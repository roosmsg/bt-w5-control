#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod device;
mod i18n;
mod protocol;
mod updates;
mod winutil;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use device::{Device, Event};
use i18n::Lang;
use protocol::{Address, BtState, Discovered, PairedDevice};

slint::include_modules!();

/// The dongle restarts its Bluetooth stack after "forget all"; wait this long before re-reading.
const RESET_DELAY: Duration = Duration::from_secs(3);
const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the "verified and saved" notice stays visible.
const SAVED_NOTICE: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Model {
    paired: Vec<PairedDevice>,
    found: Vec<Discovered>,
    bt: Option<BtState>,
    firmware: String,
    driver: String,
    scanning: bool,
    /// Codec bits of the connected headphones, see `protocol::codec`.
    codec: Option<u32>,
}

struct App {
    ui: Weak<AppWindow>,
    device: Device,
    lang: Lang,
    model: Mutex<Model>,
    scan_generation: AtomicU64,
    dry_run: bool,
}

impl App {
    /// Runs `f` on the UI thread.
    fn ui(&self, f: impl FnOnce(&AppWindow) + Send + 'static) {
        let weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                f(&ui);
            }
        });
    }

    fn message(&self, text: String) {
        self.ui(move |ui| ui.set_message(text.into()));
    }

    /// Shows `text`, then clears it after `duration` unless another message replaced it.
    fn message_for(&self, text: String, duration: Duration) {
        self.message(text.clone());
        let weak = self.ui.clone();
        thread::spawn(move || {
            thread::sleep(duration);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade()
                    && ui.get_message() == text.as_str()
                {
                    ui.set_message(SharedString::new());
                }
            });
        });
    }

    fn busy(&self, busy: bool) {
        self.ui(move |ui| ui.set_busy(busy));
    }

    /// Runs a blocking action on a background thread.
    fn spawn(self: &Arc<Self>, f: impl FnOnce(&Arc<App>) + Send + 'static) {
        let app = Arc::clone(self);
        thread::spawn(move || f(&app));
    }

    fn on_event(self: &Arc<Self>, event: Event) {
        match event {
            Event::Opened(product) => {
                self.ui(move |ui| {
                    ui.set_connected(true);
                    ui.set_model(format!("{product} (AYGO4)").into());
                });
                self.refresh();
                self.apply_audio_defaults();
            }
            Event::Closed => {
                {
                    let mut m = self.model.lock().unwrap();
                    m.paired.clear();
                    m.found.clear();
                    m.bt = None;
                    m.firmware.clear();
                    m.scanning = false;
                }
                self.ui(|ui| {
                    ui.set_connected(false);
                    ui.set_serial(SharedString::new());
                    ui.set_firmware(SharedString::new());
                    ui.set_max_paired(0);
                });
                self.push_bluetooth();
            }
            Event::State(state) => {
                let (reload, read_codec) = {
                    let mut m = self.model.lock().unwrap();
                    let connected = protocol::state_is_connected(state.state);
                    let unknown = state
                        .address
                        .is_some_and(|a| !m.paired.iter().any(|p| p.address == a));
                    // The dongle repeats its state every ~20 s; only a change (connect, music <->
                    // call) can change the codec, so read it again only then.
                    let changed =
                        m.bt.as_ref()
                            .is_none_or(|b| b.state != state.state || b.address != state.address);
                    if !connected {
                        m.codec = None;
                    }
                    m.bt = Some(state);
                    (
                        connected && unknown,
                        connected && (changed || m.codec.is_none()),
                    )
                };
                if reload {
                    self.reload_paired();
                }
                if read_codec {
                    self.read_codec();
                }
                self.push_bluetooth();
            }
            Event::Discovered(found) => {
                {
                    let mut m = self.model.lock().unwrap();
                    if !m.scanning || m.found.iter().any(|f| f.address == found.address) {
                        return;
                    }
                    m.found.push(found);
                }
                self.push_bluetooth();
            }
        }
    }

    fn refresh(&self) {
        self.busy(true);
        let firmware = self
            .device
            .call(protocol::get_firmware_string())
            .ok()
            .and_then(|f| protocol::parse_firmware_string(&f))
            .unwrap_or_default();
        let serial = self
            .device
            .call(protocol::get_serial())
            .ok()
            .and_then(|f| protocol::parse_serial(&f))
            .unwrap_or_default();
        let max_paired = self
            .device
            .call(protocol::get_feature(protocol::feature::MAX_PAIRED))
            .ok()
            .and_then(|f| protocol::parse_feature_byte(&f))
            .unwrap_or(0);
        let state = self
            .device
            .call(protocol::get_state())
            .ok()
            .and_then(|f| protocol::parse_state(&f));
        {
            let mut m = self.model.lock().unwrap();
            m.firmware = firmware.clone();
            if state.is_some() {
                m.bt = state;
            }
        }
        // Fixed behaviour: the dongle always reconnects to known headphones by itself.
        if !self.dry_run {
            ensure_auto_reconnect(&self.device);
        }
        self.reload_paired();
        self.read_codec();
        self.ui(move |ui| {
            ui.set_firmware(firmware.into());
            ui.set_serial(serial.into());
            ui.set_max_paired(i32::from(max_paired));
        });
        self.push_bluetooth();
        self.busy(false);
    }

    /// Reads the codec of the connected headphones (none while nothing is connected).
    fn read_codec(&self) {
        let connected = self
            .model
            .lock()
            .unwrap()
            .bt
            .as_ref()
            .is_some_and(|s| protocol::state_is_connected(s.state));
        let codec = if connected {
            self.device
                .call(protocol::get_feature(protocol::feature::CODEC_CURRENT))
                .ok()
                .and_then(|f| protocol::parse_feature_value(&f))
                .filter(|&bits| bits != 0)
        } else {
            None
        };
        self.model.lock().unwrap().codec = codec;
    }

    fn reload_paired(&self) {
        let count = self
            .device
            .call(protocol::get_num_paired())
            .ok()
            .and_then(|f| protocol::parse_num_paired(&f))
            .unwrap_or(0);
        let paired: Vec<PairedDevice> = (0..count)
            .filter_map(|i| self.device.call(protocol::get_paired_details(i)).ok())
            .filter_map(|f| protocol::parse_paired_details(&f))
            .filter(|p| p.valid)
            .collect();
        self.model.lock().unwrap().paired = paired;
    }

    fn push_bluetooth(&self) {
        let (rows, found, status, scanning, searching) = {
            let m = self.model.lock().unwrap();
            let connected_addr =
                m.bt.as_ref()
                    .filter(|s| protocol::state_is_connected(s.state))
                    .and_then(|s| s.address);
            let rows: Vec<PairedRow> = m
                .paired
                .iter()
                .map(|p| PairedRow {
                    name: p.name.clone().into(),
                    address: protocol::format_address(&p.address).into(),
                    connected: connected_addr == Some(p.address),
                    codec: match (connected_addr == Some(p.address), m.codec) {
                        (true, Some(bits)) => self.lang.codec(bits).into(),
                        _ => SharedString::new(),
                    },
                })
                .collect();
            let found: Vec<FoundRow> = m
                .found
                .iter()
                .map(|f| FoundRow {
                    name: f.name.clone().into(),
                    address: protocol::format_address(&f.address).into(),
                })
                .collect();
            // Only after "Pair new device" was pressed (m.scanning) and while the dongle confirms
            // it is searching for new devices. The hint then replaces the status line.
            let searching = m.scanning
                && m.bt
                    .as_ref()
                    .is_some_and(|s| s.state == protocol::STATE_INQUIRING);
            let status = m
                .bt
                .as_ref()
                // No status line while searching (normal) or connected (the paired list shows
                // which headphones are connected and with which codec).
                .filter(|s| {
                    s.state != protocol::STATE_INQUIRING && !protocol::state_is_connected(s.state)
                })
                .map(|s| self.lang.state(s.state))
                .unwrap_or_default();
            (rows, found, status, m.scanning, searching)
        };
        self.ui(move |ui| {
            ui.set_paired(ModelRc::new(VecModel::from(rows)));
            ui.set_found(ModelRc::new(VecModel::from(found)));
            ui.set_bt_status(status.into());
            ui.set_scanning(scanning);
            ui.set_device_searching(searching);
        });
    }

    fn apply_audio_defaults(&self) {
        let summary = audio::apply_defaults(!self.dry_run);
        let describe = |f: Option<audio::Format>| {
            f.map(|f| {
                let channels = if f.channels == 1 { "mono" } else { "stereo" };
                format!(
                    "{}-bit / {} kHz · {channels}",
                    f.bits,
                    f64::from(f.rate) / 1000.0
                )
            })
            .unwrap_or_default()
        };
        let headphones = describe(summary.headphones);
        let microphone = describe(summary.microphone);
        let driver = summary.driver_version.clone().unwrap_or_default();
        self.model.lock().unwrap().driver = driver.clone();
        self.ui(move |ui| {
            ui.set_headphones(headphones.into());
            ui.set_microphone(microphone.into());
            ui.set_driver(driver.into());
        });
        if let Some(problem) = summary.problems.first() {
            self.message(self.lang.audio_problem(problem));
        }
    }

    fn check_updates(self: &Arc<Self>) {
        let checking = self.lang.checking();
        self.ui(move |ui| {
            ui.set_firmware_update(checking.clone().into());
            ui.set_driver_update(checking.into());
        });
        self.spawn(|app| {
            let (firmware, driver) = {
                let m = app.model.lock().unwrap();
                (m.firmware.clone(), m.driver.clone())
            };
            // Without a device only the latest published version can be shown.
            let firmware = if firmware.is_empty() {
                match updates::latest_firmware() {
                    Ok(v) => v,
                    Err(e) => app.lang.update(&updates::Status::Unknown(e)),
                }
            } else {
                app.lang.update(&updates::check_firmware(&firmware))
            };
            let driver = if driver.is_empty() {
                String::new()
            } else {
                app.lang.update(&updates::check_driver(&driver))
            };
            app.ui(move |ui| {
                ui.set_firmware_update(firmware.into());
                ui.set_driver_update(driver.into());
            });
        });
    }

    fn download_firmware(self: &Arc<Self>) {
        self.download(updates::download_firmware_installer);
    }

    fn download_driver(self: &Arc<Self>) {
        self.download(updates::download_driver_installer);
    }

    /// Download only: the verified file is saved to Downloads and shown in Explorer,
    /// never run.
    fn download(
        self: &Arc<Self>,
        fetch: fn(&std::path::Path) -> Result<std::path::PathBuf, String>,
    ) {
        self.busy(true);
        self.spawn(move |app| {
            app.message(app.lang.downloading());
            match winutil::downloads_dir().and_then(|dir| fetch(&dir)) {
                Ok(path) => {
                    app.message_for(app.lang.saved(&path.display().to_string()), SAVED_NOTICE);
                    winutil::reveal_in_explorer(&path);
                }
                Err(e) => app.message(app.lang.failed(&e)),
            }
            app.busy(false);
        });
    }

    fn reset_list(self: &Arc<Self>) {
        self.busy(true);
        self.spawn(|app| {
            let (title, text) = app.lang.confirm_reset();
            if !winutil::confirm(&title, &text) {
                app.busy(false);
                return;
            }
            // Device errors are not shown; the state display reflects the outcome.
            if app.device.call(protocol::forget_all()).is_ok() {
                thread::sleep(RESET_DELAY);
                app.refresh();
                app.message(app.lang.reset_done());
            }
            app.busy(false);
        });
    }

    fn paired_address(&self, index: i32) -> Option<Address> {
        let m = self.model.lock().unwrap();
        usize::try_from(index)
            .ok()
            .and_then(|i| m.paired.get(i))
            .map(|p| p.address)
    }

    fn found_address(&self, index: i32) -> Option<Address> {
        let m = self.model.lock().unwrap();
        usize::try_from(index)
            .ok()
            .and_then(|i| m.found.get(i))
            .map(|f| f.address)
    }

    fn send(self: &Arc<Self>, request: protocol::Request) {
        self.spawn(move |app| {
            // Device errors are not shown; state events update the UI instead.
            let _ = app.device.call(request);
        });
    }

    fn start_scan(self: &Arc<Self>) {
        {
            let mut m = self.model.lock().unwrap();
            m.found.clear();
            m.scanning = true;
        }
        self.push_bluetooth();
        let generation = self.scan_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn(move |app| {
            if app.device.call(protocol::start_scan()).is_err() {
                app.model.lock().unwrap().scanning = false;
                app.push_bluetooth();
                return;
            }
            thread::sleep(SCAN_TIMEOUT);
            if app.scan_generation.load(Ordering::SeqCst) == generation
                && app.model.lock().unwrap().scanning
            {
                app.stop_scan();
            }
        });
    }

    fn stop_scan(self: &Arc<Self>) {
        self.scan_generation.fetch_add(1, Ordering::SeqCst);
        {
            let mut m = self.model.lock().unwrap();
            m.scanning = false;
            m.found.clear();
        }
        self.push_bluetooth();
        self.send(protocol::stop_scan());
    }

    fn pair(self: &Arc<Self>, index: i32) {
        let Some(address) = self.found_address(index) else {
            return;
        };
        self.scan_generation.fetch_add(1, Ordering::SeqCst);
        {
            let mut m = self.model.lock().unwrap();
            m.scanning = false;
            m.found.clear();
        }
        self.push_bluetooth();
        self.send(protocol::pair(&address));
    }
}

/// Reads the dongle's auto-reconnect setting and turns it on when it is off. Returns the value
/// read afterwards (`None` when the dongle did not answer). The dongle can answer the set late
/// (seen right after a list reset), so the result is always read back.
fn ensure_auto_reconnect(device: &Device) -> Option<bool> {
    let read = || {
        device
            .call(protocol::get_feature(protocol::feature::AUTO_RECONNECT))
            .ok()
            .and_then(|f| protocol::parse_feature_byte(&f))
            .map(|v| v != 0)
    };
    if read() == Some(true) {
        return Some(true);
    }
    let _ = device.call(protocol::set_feature(protocol::feature::AUTO_RECONNECT, 1));
    read()
}

/// Logon mode: apply the audio defaults once the dongle's endpoints exist, make sure the dongle
/// reconnects automatically, then exit.
fn run_startup() {
    for _ in 0..12 {
        let summary = audio::apply_defaults(true);
        if summary.headphones.is_some() || summary.microphone.is_some() {
            break;
        }
        thread::sleep(Duration::from_secs(5));
    }
    let (tx, rx) = mpsc::channel();
    let device = Device::spawn(move |event| {
        let _ = tx.send(event);
    });
    if matches!(
        rx.recv_timeout(Duration::from_secs(10)),
        Ok(Event::Opened(_))
    ) {
        ensure_auto_reconnect(&device);
    }
}

/// Read-only diagnostics: queries the dongle and the audio endpoints, prints, writes nothing.
fn run_probe() {
    for line in audio::inspect() {
        println!("{line}");
    }
    let driver = audio::apply_defaults(false)
        .driver_version
        .unwrap_or_default();
    println!("driver update: {:?}", updates::check_driver(&driver));
    println!("downloads dir: {:?}", winutil::downloads_dir());
    let (tx, rx) = mpsc::channel();
    let device = Device::spawn(move |event| {
        let _ = tx.send(event);
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Event::Opened(product)) => println!("hid: opened {product}"),
        other => {
            println!("hid: not opened ({other:?})");
            return;
        }
    }
    let show = |name: &str, request: protocol::Request| match device.call(request) {
        Ok(frame) => {
            println!("{name}: {:02X?}", frame.payload);
            Some(frame)
        }
        Err(e) => {
            println!("{name}: {e}");
            None
        }
    };
    if let Some(f) = show("firmware", protocol::get_firmware_string()) {
        let version = protocol::parse_firmware_string(&f).unwrap_or_default();
        println!("  = {version:?}");
        println!("firmware update: {:?}", updates::check_firmware(&version));
    }
    if let Some(f) = show("serial", protocol::get_serial()) {
        println!("  = {:?}", protocol::parse_serial(&f));
    }
    if let Some(f) = show(
        "max paired",
        protocol::get_feature(protocol::feature::MAX_PAIRED),
    ) {
        println!("  = {:?}", protocol::parse_feature_byte(&f));
    }
    if let Some(f) = show(
        "auto reconnect",
        protocol::get_feature(protocol::feature::AUTO_RECONNECT),
    ) {
        println!("  = {:?}", protocol::parse_feature_byte(&f));
    }
    if let Some(f) = show("state", protocol::get_state()) {
        println!("  = {:?}", protocol::parse_state(&f));
    }
    if let Some(f) = show(
        "codec",
        protocol::get_feature(protocol::feature::CODEC_CURRENT),
    ) {
        let bits = protocol::parse_feature_value(&f).unwrap_or_default();
        println!("  = 0x{bits:X} {}", i18n::Lang::detect().codec(bits));
    }
    let count = show("paired count", protocol::get_num_paired())
        .and_then(|f| protocol::parse_num_paired(&f))
        .unwrap_or(0);
    for i in 0..count {
        if let Some(f) = show("paired", protocol::get_paired_details(i)) {
            println!("  = {:?}", protocol::parse_paired_details(&f));
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    if std::env::args().any(|a| a == "--startup") {
        run_startup();
        return Ok(());
    }
    if std::env::args().any(|a| a == "--probe") {
        run_probe();
        return Ok(());
    }
    if std::env::args().any(|a| a == "--uninstall") {
        // Undo what the app sets: the logon entry and the audio-enhancements bypass.
        println!("autostart removed: {:?}", winutil::remove_autostart());
        for line in audio::restore_enhancements() {
            println!("{line}");
        }
        return Ok(());
    }
    let Some(_instance) = winutil::SingleInstance::acquire() else {
        return Ok(());
    };
    // `--dry-run` shows the UI without registering autostart or writing audio settings.
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    if !dry_run {
        let _ = winutil::ensure_autostart();
    }

    let lang = Lang::detect();
    let ui = AppWindow::new()?;
    ui.global::<Tr>().set_t(lang.texts());

    let (events_tx, events_rx) = mpsc::channel();
    let device = Device::spawn(move |event| {
        let _ = events_tx.send(event);
    });
    let app = Arc::new(App {
        ui: ui.as_weak(),
        device,
        lang,
        model: Mutex::new(Model::default()),
        scan_generation: AtomicU64::new(0),
        dry_run,
    });

    {
        let app = Arc::clone(&app);
        thread::spawn(move || {
            for event in events_rx {
                app.on_event(event);
            }
        });
    }
    // The audio defaults do not depend on the HID channel, so apply them right away too.
    app.spawn(|app| app.apply_audio_defaults());

    let a = Arc::clone(&app);
    ui.on_check_updates(move || a.check_updates());
    let a = Arc::clone(&app);
    ui.on_download_firmware(move || a.download_firmware());
    let a = Arc::clone(&app);
    ui.on_download_driver(move || a.download_driver());
    let a = Arc::clone(&app);
    ui.on_reset_list(move || a.reset_list());
    let a = Arc::clone(&app);
    ui.on_connect_device(move |i| {
        if let Some(address) = a.paired_address(i) {
            a.send(protocol::connect(&address));
        }
    });
    let a = Arc::clone(&app);
    ui.on_disconnect_device(move |i| {
        if let Some(address) = a.paired_address(i) {
            a.send(protocol::disconnect(&address));
        }
    });
    let a = Arc::clone(&app);
    ui.on_start_scan(move || a.start_scan());
    let a = Arc::clone(&app);
    ui.on_stop_scan(move || a.stop_scan());
    let a = Arc::clone(&app);
    ui.on_pair_device(move |i| a.pair(i));

    ui.set_creative_running(winutil::process_running("Creative.App.exe"));
    let weak = ui.as_weak();
    let creative_timer = slint::Timer::default();
    creative_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(5),
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_creative_running(winutil::process_running("Creative.App.exe"));
            }
        },
    );

    ui.run()
}
