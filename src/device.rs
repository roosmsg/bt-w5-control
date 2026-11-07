//! HID connection to the BT-W5. One worker thread owns the handle: it sends queued requests,
//! waits for the matching reply, and forwards every other frame as an [`Event`].

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};

use crate::protocol::{self, Frame, Request};

const READ_POLL_MS: i32 = 200;
const REOPEN_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum Event {
    /// The control channel was opened; carries the USB product string.
    Opened(String),
    Closed,
    State(protocol::BtState),
    Discovered(protocol::Discovered),
}

#[derive(Debug)]
pub enum Error {
    NotConnected,
    Timeout,
    Refused(u8),
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotConnected => write!(f, "device not connected"),
            Error::Timeout => write!(f, "device did not answer"),
            Error::Refused(status) => write!(f, "device refused the command (status {status})"),
            Error::Io(e) => write!(f, "HID error: {e}"),
        }
    }
}

enum Message {
    Call {
        request: Request,
        timeout: Duration,
        reply: Sender<Result<Frame, Error>>,
    },
}

#[derive(Clone)]
pub struct Device {
    tx: Sender<Message>,
}

impl Device {
    pub fn spawn(on_event: impl Fn(Event) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("hid".into())
            .spawn(move || Worker::new(rx, on_event).run())
            .expect("failed to start HID thread");
        Self { tx }
    }

    pub fn call(&self, request: Request) -> Result<Frame, Error> {
        self.call_with_timeout(request, DEFAULT_TIMEOUT)
    }

    pub fn call_with_timeout(&self, request: Request, timeout: Duration) -> Result<Frame, Error> {
        let (reply, result) = mpsc::channel();
        self.tx
            .send(Message::Call {
                request,
                timeout,
                reply,
            })
            .map_err(|_| Error::NotConnected)?;
        result
            .recv_timeout(timeout + Duration::from_secs(2))
            .unwrap_or(Err(Error::Timeout))
    }
}

struct Worker<F> {
    rx: Receiver<Message>,
    on_event: F,
    api: Option<HidApi>,
    device: Option<HidDevice>,
    last_open_attempt: Option<Instant>,
}

impl<F: Fn(Event)> Worker<F> {
    fn new(rx: Receiver<Message>, on_event: F) -> Self {
        Self {
            rx,
            on_event,
            api: None,
            device: None,
            last_open_attempt: None,
        }
    }

    fn run(mut self) {
        loop {
            if self.device.is_none() {
                self.try_open();
            }

            let message = if self.device.is_some() {
                match self.rx.try_recv() {
                    Ok(m) => Some(m),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            } else {
                match self.rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };

            match message {
                Some(Message::Call {
                    request,
                    timeout,
                    reply,
                }) => {
                    let result = self.execute(&request, timeout);
                    let _ = reply.send(result);
                }
                None => {
                    if self.device.is_some() {
                        self.read_once(READ_POLL_MS);
                    }
                }
            }
        }
    }

    fn try_open(&mut self) {
        if self
            .last_open_attempt
            .is_some_and(|t| t.elapsed() < REOPEN_INTERVAL)
        {
            return;
        }
        self.last_open_attempt = Some(Instant::now());

        let api = match self.api.as_mut() {
            Some(api) => {
                if api.refresh_devices().is_err() {
                    return;
                }
                api
            }
            None => match HidApi::new() {
                Ok(api) => self.api.insert(api),
                Err(_) => return,
            },
        };

        let Some(info) = api.device_list().find(|d| {
            d.vendor_id() == protocol::VENDOR_ID
                && d.product_id() == protocol::PRODUCT_ID
                && d.usage_page() == protocol::USAGE_PAGE
                && d.usage() == protocol::USAGE
        }) else {
            return;
        };
        let product = info
            .product_string()
            .unwrap_or("Creative BT-W5")
            .to_string();
        if let Ok(device) = api.open_path(info.path()) {
            self.device = Some(device);
            (self.on_event)(Event::Opened(product));
        }
    }

    fn close(&mut self) {
        if self.device.take().is_some() {
            (self.on_event)(Event::Closed);
        }
    }

    fn execute(&mut self, request: &Request, timeout: Duration) -> Result<Frame, Error> {
        let Some(device) = self.device.as_ref() else {
            return Err(Error::NotConnected);
        };
        if let Err(e) = device.write(&request.frame.to_report()) {
            self.close();
            return Err(Error::Io(e.to_string()));
        }

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let wait = remaining.as_millis().min(READ_POLL_MS as u128) as i32;
            let Some(frame) = self.read_once(wait.max(1)) else {
                if self.device.is_none() {
                    return Err(Error::NotConnected);
                }
                continue;
            };
            match request.expect.matches(&frame) {
                Some(Ok(())) => return Ok(frame),
                Some(Err(status)) => return Err(Error::Refused(status)),
                None => {}
            }
        }
    }

    /// Reads one report; unsolicited frames are dispatched as events. Returns the frame so a
    /// pending request can check it too.
    fn read_once(&mut self, timeout_ms: i32) -> Option<Frame> {
        let device = self.device.as_ref()?;
        let mut buf = [0u8; protocol::REPORT_LEN];
        match device.read_timeout(&mut buf, timeout_ms) {
            Ok(0) => None,
            Ok(n) => {
                let frame = Frame::from_report(&buf[..n])?;
                self.dispatch(&frame);
                Some(frame)
            }
            Err(_) => {
                self.close();
                None
            }
        }
    }

    fn dispatch(&self, frame: &Frame) {
        if frame.cmd != protocol::CMD_BT {
            return;
        }
        match frame.op() {
            Some(protocol::bt_op::STATE) => {
                if let Some(state) = protocol::parse_state(frame) {
                    (self.on_event)(Event::State(state));
                }
            }
            Some(protocol::bt_op::INQUIRY_DATA) => {
                if let Some(found) = protocol::parse_inquiry_data(frame) {
                    (self.on_event)(Event::Discovered(found));
                }
            }
            _ => {}
        }
    }
}
