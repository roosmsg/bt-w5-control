//! User-facing text in English and Dutch, chosen from the Windows display language.

use crate::Texts;
use crate::updates::Status;

#[derive(Clone, Copy)]
pub struct Lang {
    nl: bool,
}

impl Lang {
    pub fn detect() -> Self {
        Self {
            nl: crate::winutil::ui_language() == "nl",
        }
    }

    fn t(&self, en: &str, nl: &str) -> String {
        if self.nl { nl } else { en }.to_string()
    }

    pub fn texts(&self) -> Texts {
        let t = |en: &str, nl: &str| -> slint::SharedString { self.t(en, nl).into() };
        Texts {
            device: t("Device", "Apparaat"),
            model: t("Model", "Model"),
            serial: t("Serial number", "Serienummer"),
            firmware: t("Firmware", "Firmware"),
            driver: t("Audio driver", "Audiostuurprogramma"),
            headphones: t("Headphones", "Koptelefoon"),
            microphone: t("Microphone", "Microfoon"),
            not_found: t(
                "BT-W5 not found. Plug in the dongle.",
                "BT-W5 niet gevonden. Sluit de dongle aan.",
            ),
            updates: t("Updates", "Updates"),
            check_updates: t("Check for updates", "Controleren op updates"),
            download_firmware: t("Download firmware", "Download firmware"),
            download_driver: t("Download audio driver", "Download audio driver"),
            bluetooth: t("Bluetooth", "Bluetooth"),
            paired_devices: t("Paired devices", "Gekoppelde apparaten"),
            none_paired: t("No paired devices", "Geen gekoppelde apparaten"),
            connect: t("Connect", "Verbinden"),
            disconnect: t("Disconnect", "Verbreken"),
            connected: t("Connected", "Verbonden"),
            pair_new: t("Pair new device", "Nieuw apparaat koppelen"),
            stop_search: t("Stop searching", "Stoppen met zoeken"),
            searching: t(
                "Searching… Put your headphones in pairing mode.",
                "Zoeken… Zet je koptelefoon in koppelmodus.",
            ),
            pair: t("Pair", "Koppelen"),
            reset_list: t("Reset Bluetooth list", "Bluetooth-lijst wissen"),
            creative_warning: t(
                "The Creative App is running. Close or uninstall it: two apps controlling the dongle interfere with each other.",
                "De Creative App draait. Sluit of verwijder die: twee apps die de dongle aansturen zitten elkaar in de weg.",
            ),
        }
    }

    pub fn state(&self, state: u8) -> String {
        let (en, nl) = match state {
            0 => ("Starting", "Opstarten"),
            1 => ("Powered off", "Uitgeschakeld"),
            2 => ("Test mode", "Testmodus"),
            3 => ("Idle", "Inactief"),
            4 => ("Waiting for connection", "Wacht op verbinding"),
            5 => ("Discoverable", "Zichtbaar"),
            6 => ("Connecting…", "Bezig met verbinden…"),
            7 => ("Searching…", "Zoeken…"),
            8 => ("Connected", "Verbonden"),
            9 => ("Calling device…", "Apparaat oproepen…"),
            10 => ("Disconnected", "Niet verbonden"),
            11 => ("Connected (music)", "Verbonden (muziek)"),
            12 => ("Connected (call)", "Verbonden (gesprek)"),
            13 => ("Connection failed", "Verbinden mislukt"),
            14 => ("Connected (LE Audio)", "Verbonden (LE Audio)"),
            15 => (
                "Connected (LE Audio, call)",
                "Verbonden (LE Audio, gesprek)",
            ),
            16 => ("Broadcasting (Auracast)", "Uitzenden (Auracast)"),
            _ => ("Unknown state", "Onbekende status"),
        };
        self.t(en, nl)
    }

    pub fn update(&self, status: &Status) -> String {
        match status {
            Status::UpToDate(v) => {
                if self.nl {
                    format!("Up-to-date ({v})")
                } else {
                    format!("Up to date ({v})")
                }
            }
            Status::Available { installed, latest } => {
                if self.nl {
                    format!("Update beschikbaar: {installed} → {latest} (niet geïnstalleerd)")
                } else {
                    format!("Update available: {installed} → {latest} (not installed)")
                }
            }
            Status::Unknown(e) => {
                if self.nl {
                    format!("Controle mislukt: {e}")
                } else {
                    format!("Check failed: {e}")
                }
            }
        }
    }

    /// Name of the codec bit reported by the dongle.
    pub fn codec(&self, bits: u32) -> String {
        use crate::protocol::codec::*;
        match bits {
            SBC => "SBC".into(),
            FASTSTREAM => "FastStream".into(),
            APTX => "aptX".into(),
            APTX_LOW_LATENCY => "aptX Low Latency".into(),
            APTX_HD => "aptX HD".into(),
            APTX_ADAPTIVE_LOW_LATENCY => self.t(
                "aptX Adaptive (low latency)",
                "aptX Adaptive (lage latentie)",
            ),
            APTX_ADAPTIVE_HIGH_QUALITY => self.t(
                "aptX Adaptive (high quality)",
                "aptX Adaptive (hoge kwaliteit)",
            ),
            APTX_LOSSLESS => "aptX Lossless".into(),
            APTX_LITE => "aptX Lite".into(),
            LC3 => "LC3 (LE Audio)".into(),
            other => format!("codec 0x{other:X}"),
        }
    }

    pub fn checking(&self) -> String {
        self.t("Checking…", "Controleren…")
    }

    pub fn confirm_reset(&self) -> (String, String) {
        (
            self.t("Reset Bluetooth list", "Bluetooth-lijst wissen"),
            self.t(
                "Forget all paired devices on the BT-W5? You will have to pair your headphones again.",
                "Alle gekoppelde apparaten op de BT-W5 vergeten? Je moet je koptelefoon daarna opnieuw koppelen.",
            ),
        )
    }

    pub fn reset_done(&self) -> String {
        self.t("Bluetooth list cleared.", "Bluetooth-lijst gewist.")
    }

    pub fn downloading(&self) -> String {
        self.t("Downloading and verifying…", "Downloaden en controleren…")
    }

    pub fn saved(&self, path: &str) -> String {
        if self.nl {
            format!("Gecontroleerd en opgeslagen (niet uitgevoerd): {path}")
        } else {
            format!("Verified and saved (not run): {path}")
        }
    }

    pub fn failed(&self, what: &str) -> String {
        if self.nl {
            format!("Mislukt: {what}")
        } else {
            format!("Failed: {what}")
        }
    }

    pub fn audio_problem(&self, what: &str) -> String {
        if self.nl {
            format!("Audio-instelling niet toegepast: {what}")
        } else {
            format!("Audio setting not applied: {what}")
        }
    }
}
