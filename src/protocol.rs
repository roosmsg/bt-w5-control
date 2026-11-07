//! Creative's HID control protocol as used by the BT-W5 (product code AYGO4).
//!
//! Report layout on the vendor collection (usage page 0xFF01, usage 1):
//! `[0x03 report id] [0x5A] [cmd] [len] [payload; len] [zero padding to 65 bytes]`.

pub const VENDOR_ID: u16 = 0x041E;
pub const PRODUCT_ID: u16 = 0x3130;
pub const USAGE_PAGE: u16 = 0xFF01;
pub const USAGE: u16 = 0x0001;
pub const REPORT_ID: u8 = 0x03;
pub const REPORT_LEN: usize = 65;
pub const START: u8 = 0x5A;

/// Command ids used here.
pub const CMD_ACK: u8 = 0x02;
pub const CMD_DEVICE_INFO: u8 = 0x09;
pub const CMD_BT: u8 = 0x6B;

/// `DeviceInformationV2` operations.
pub mod info_op {
    pub const FIRMWARE_STRING: u8 = 2;
    pub const SERIAL: u8 = 3;
}

/// Bluetooth control operations.
pub mod bt_op {
    pub const NUM_PAIRED: u8 = 1;
    pub const PAIRED_DETAILS: u8 = 2;
    pub const CONNECTION_ACTION: u8 = 3;
    pub const INQUIRY_MODE: u8 = 4;
    pub const INQUIRY_DATA: u8 = 5;
    pub const STATE: u8 = 6;
    pub const FORGET: u8 = 7;
    pub const GET_FEATURE: u8 = 9;
    pub const SET_FEATURE: u8 = 10;
    pub const STOP_INQUIRY: u8 = 11;
}

/// Bluetooth feature ids.
pub mod feature {
    pub const AUTO_RECONNECT: u8 = 0;
    pub const MAX_PAIRED: u8 = 1;
    /// The A2DP codec in use, one bit of [`super::codec`].
    pub const CODEC_CURRENT: u8 = 3;
}

/// Bluetooth codec bits, as reported by [`feature::CODEC_CURRENT`].
pub mod codec {
    pub const SBC: u32 = 0x1;
    pub const FASTSTREAM: u32 = 0x2;
    pub const APTX: u32 = 0x4;
    pub const APTX_LOW_LATENCY: u32 = 0x8;
    pub const APTX_HD: u32 = 0x10;
    pub const APTX_ADAPTIVE_LOW_LATENCY: u32 = 0x20;
    pub const APTX_ADAPTIVE_HIGH_QUALITY: u32 = 0x40;
    pub const APTX_LOSSLESS: u32 = 0x80;
    pub const APTX_LITE: u32 = 0x100;
    pub const LC3: u32 = 0x200;
}

pub type Address = [u8; 6];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub cmd: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(cmd: u8, payload: &[u8]) -> Self {
        Self {
            cmd,
            payload: payload.to_vec(),
        }
    }

    /// First payload byte: the operation for requests and replies, the acknowledged command
    /// for `CMD_ACK` frames.
    pub fn op(&self) -> Option<u8> {
        self.payload.first().copied()
    }

    pub fn to_report(&self) -> [u8; REPORT_LEN] {
        let mut report = [0u8; REPORT_LEN];
        report[0] = REPORT_ID;
        report[1] = START;
        report[2] = self.cmd;
        let len = self.payload.len().min(REPORT_LEN - 4);
        report[3] = len as u8;
        report[4..4 + len].copy_from_slice(&self.payload[..len]);
        report
    }

    /// Parses an input report, with or without the leading report id byte.
    pub fn from_report(buf: &[u8]) -> Option<Self> {
        let body = match buf {
            [REPORT_ID, START, ..] => &buf[1..],
            [START, ..] => buf,
            _ => return None,
        };
        if body.len() < 3 {
            return None;
        }
        let cmd = body[1];
        let len = usize::from(body[2]).min(body.len() - 3);
        Some(Self {
            cmd,
            payload: body[3..3 + len].to_vec(),
        })
    }

    /// For an acknowledgement frame: (acknowledged command, status, operation).
    pub fn ack(&self) -> Option<(u8, u8, u8)> {
        if self.cmd != CMD_ACK || self.payload.len() < 3 {
            return None;
        }
        Some((self.payload[0], self.payload[1], self.payload[2]))
    }
}

/// What a request waits for.
#[derive(Clone, Copy, Debug)]
pub enum Expect {
    /// A reply frame with this command whose first payload byte is `op`.
    Data { cmd: u8, op: u8 },
    /// An acknowledgement of `cmd`/`op`; a non-zero status is an error.
    Ack { cmd: u8, op: u8 },
}

impl Expect {
    /// `Some(Ok)` when `frame` completes the request, `Some(Err)` when the device refused it.
    pub fn matches(&self, frame: &Frame) -> Option<Result<(), u8>> {
        match *self {
            Expect::Data { cmd, op } => {
                if frame.cmd == cmd && frame.op() == Some(op) {
                    return Some(Ok(()));
                }
                match frame.ack() {
                    Some((c, status, o)) if c == cmd && o == op && status != 0 => Some(Err(status)),
                    _ => None,
                }
            }
            Expect::Ack { cmd, op } => match frame.ack() {
                Some((c, 0, o)) if c == cmd && o == op => Some(Ok(())),
                Some((c, status, o)) if c == cmd && o == op => Some(Err(status)),
                _ => None,
            },
        }
    }
}

pub struct Request {
    pub frame: Frame,
    pub expect: Expect,
}

fn data(cmd: u8, payload: &[u8]) -> Request {
    Request {
        frame: Frame::new(cmd, payload),
        expect: Expect::Data {
            cmd,
            op: payload[0],
        },
    }
}

fn acked(cmd: u8, payload: &[u8]) -> Request {
    Request {
        frame: Frame::new(cmd, payload),
        expect: Expect::Ack {
            cmd,
            op: payload[0],
        },
    }
}

pub fn get_firmware_string() -> Request {
    data(CMD_DEVICE_INFO, &[info_op::FIRMWARE_STRING])
}

pub fn get_serial() -> Request {
    data(CMD_DEVICE_INFO, &[info_op::SERIAL])
}

pub fn get_num_paired() -> Request {
    data(CMD_BT, &[bt_op::NUM_PAIRED])
}

pub fn get_paired_details(index: u8) -> Request {
    data(CMD_BT, &[bt_op::PAIRED_DETAILS, index])
}

pub fn get_state() -> Request {
    data(CMD_BT, &[bt_op::STATE])
}

pub fn get_feature(kind: u8) -> Request {
    data(CMD_BT, &[bt_op::GET_FEATURE, kind])
}

/// Sets a one-byte feature, e.g. [`feature::AUTO_RECONNECT`] = 1.
pub fn set_feature(kind: u8, value: u8) -> Request {
    acked(CMD_BT, &[bt_op::SET_FEATURE, kind, value])
}

/// Forgets every paired device (index 0xFF), the app's "Reset Bluetooth list".
pub fn forget_all() -> Request {
    acked(CMD_BT, &[bt_op::FORGET, 0xFF])
}

pub fn connect(address: &Address) -> Request {
    let mut payload = vec![bt_op::CONNECTION_ACTION, 0];
    payload.extend_from_slice(address);
    acked(CMD_BT, &payload)
}

pub fn disconnect(address: &Address) -> Request {
    let mut payload = vec![bt_op::CONNECTION_ACTION, 1];
    payload.extend_from_slice(address);
    acked(CMD_BT, &payload)
}

/// Starts discovery of headphones in pairing mode (`InquiryGetSinksOnly`).
pub fn start_scan() -> Request {
    acked(CMD_BT, &[bt_op::INQUIRY_MODE, 1])
}

/// Pairs and connects a discovered sink (`InquiryPairSpecificSink`).
pub fn pair(address: &Address) -> Request {
    let mut payload = vec![bt_op::INQUIRY_MODE, 2];
    payload.extend_from_slice(address);
    acked(CMD_BT, &payload)
}

pub fn stop_scan() -> Request {
    acked(CMD_BT, &[bt_op::STOP_INQUIRY])
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn address(bytes: &[u8]) -> Option<Address> {
    bytes.get(..6)?.try_into().ok()
}

pub fn format_address(address: &Address) -> String {
    address
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn parse_firmware_string(frame: &Frame) -> Option<String> {
    let p = &frame.payload;
    let n = usize::from(*p.get(1)?);
    Some(c_string(p.get(2..(2 + n).min(p.len()))?))
}

pub fn parse_serial(frame: &Frame) -> Option<String> {
    Some(c_string(frame.payload.get(1..)?))
}

pub fn parse_num_paired(frame: &Frame) -> Option<u8> {
    frame.payload.get(1).copied()
}

pub fn parse_feature_byte(frame: &Frame) -> Option<u8> {
    frame.payload.get(2).copied()
}

/// A feature value of one or more bytes, little-endian (codec bits can exceed one byte).
pub fn parse_feature_value(frame: &Frame) -> Option<u32> {
    let bytes = frame.payload.get(2..)?;
    if bytes.is_empty() {
        return None;
    }
    Some(
        bytes
            .iter()
            .take(4)
            .enumerate()
            .fold(0, |v, (i, &b)| v | u32::from(b) << (8 * i)),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedDevice {
    pub index: u8,
    pub valid: bool,
    pub address: Address,
    pub name: String,
}

pub fn parse_paired_details(frame: &Frame) -> Option<PairedDevice> {
    let p = &frame.payload;
    let size = usize::from(*p.get(9)?);
    Some(PairedDevice {
        index: *p.get(1)?,
        valid: *p.get(2)? & 0x80 != 0,
        address: address(p.get(3..)?)?,
        name: c_string(p.get(10..(10 + size).min(p.len())).unwrap_or_default()),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtState {
    /// Bluetooth state value, see [`state_is_connected`].
    pub state: u8,
    pub address: Option<Address>,
    pub name: String,
}

pub fn parse_state(frame: &Frame) -> Option<BtState> {
    let p = &frame.payload;
    let state = *p.get(1)?;
    let address = address(p.get(2..).unwrap_or_default()).filter(|a| a.iter().any(|&b| b != 0));
    let name = p
        .get(8)
        .map(|&size| {
            c_string(
                p.get(9..(9 + usize::from(size)).min(p.len()))
                    .unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    Some(BtState {
        state,
        address,
        name,
    })
}

/// Bluetooth state 7: the dongle is searching for new devices.
pub const STATE_INQUIRING: u8 = 7;

/// Connected, A2DP, HFP and LE Audio unicast states.
pub fn state_is_connected(state: u8) -> bool {
    matches!(state, 8 | 11 | 12 | 14 | 15)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovered {
    pub address: Address,
    pub name: String,
}

pub fn parse_inquiry_data(frame: &Frame) -> Option<Discovered> {
    let p = &frame.payload;
    let size = usize::from(*p.get(7)?);
    Some(Discovered {
        address: address(p.get(1..)?)?,
        name: c_string(p.get(8..(8 + size).min(p.len())).unwrap_or_default()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split('-')
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    #[test]
    fn report_matches_logged_reset_frame() {
        let report = forget_all().frame.to_report();
        assert_eq!(&report[..6], &[0x03, 0x5A, 0x6B, 0x02, 0x07, 0xFF]);
        assert!(report[6..].iter().all(|&b| b == 0));
        assert_eq!(report.len(), 65);
    }

    #[test]
    fn connect_frame_matches_log() {
        let addr = [0x9E, 0x70, 0xF1, 0x19, 0x2C, 0x27];
        let report = connect(&addr).frame.to_report();
        assert_eq!(
            &report[1..12],
            hex("5A-6B-08-03-00-9E-70-F1-19-2C-27").as_slice()
        );
    }

    #[test]
    fn parses_firmware_and_serial_from_log() {
        let fw = Frame::from_report(&hex(
            "03-5A-09-12-02-10-31-2E-32-2E-32-33-30-35-31-31-2E-31-32-32-30-00-00",
        ))
        .unwrap();
        assert!(get_firmware_string().expect.matches(&fw) == Some(Ok(())));
        assert_eq!(parse_firmware_string(&fw).unwrap(), "1.2.230511.1220");

        let serial = Frame::from_report(&hex(
            "5A-09-13-03-59-44-53-41-30-31-38-30-33-32-33-30-30-34-33-38-31-53-00",
        ))
        .unwrap();
        assert_eq!(parse_serial(&serial).unwrap(), "YDSA0180323004381S");
    }

    #[test]
    fn parses_paired_device_and_skips_ack() {
        let ack = Frame::from_report(&hex("5A-02-0A-6B-00-02-00-00-00-00-00-00-00")).unwrap();
        let req = get_paired_details(0);
        assert_eq!(req.expect.matches(&ack), None);

        let mut bytes =
            hex("5A-6B-2D-02-00-80-9E-70-F1-19-2C-27-1F-46-6F-63-61-6C-20-42-61-74-68-79-73");
        bytes.resize(48, 0);
        let frame = Frame::from_report(&bytes).unwrap();
        assert_eq!(req.expect.matches(&frame), Some(Ok(())));
        let dev = parse_paired_details(&frame).unwrap();
        assert!(dev.valid);
        assert_eq!(dev.name, "Focal Bathys");
        assert_eq!(format_address(&dev.address), "9E:70:F1:19:2C:27");
    }

    #[test]
    fn ack_completes_set_requests() {
        let ack = Frame::from_report(&hex("5A-02-0A-6B-00-03-00-00-00-00-00-00-00")).unwrap();
        let addr = [0x9E, 0x70, 0xF1, 0x19, 0x2C, 0x27];
        assert_eq!(connect(&addr).expect.matches(&ack), Some(Ok(())));
        assert_eq!(start_scan().expect.matches(&ack), None);
        let failed = Frame::from_report(&hex("5A-02-0A-6B-05-03-00")).unwrap();
        assert_eq!(connect(&addr).expect.matches(&failed), Some(Err(5)));
    }

    #[test]
    fn parses_state_frames() {
        let mut bytes =
            hex("5A-6B-29-06-08-9E-70-F1-19-2C-27-20-46-6F-63-61-6C-20-42-61-74-68-79-73");
        bytes.resize(48, 0);
        let s = parse_state(&Frame::from_report(&bytes).unwrap()).unwrap();
        assert_eq!(s.state, 8);
        assert!(state_is_connected(s.state));
        assert_eq!(s.name, "Focal Bathys");

        let short = parse_state(&Frame::from_report(&hex("5A-6B-02-06-07-00")).unwrap()).unwrap();
        assert_eq!(short.state, 7);
        assert_eq!(short.address, None);
    }

    #[test]
    fn parses_inquiry_result() {
        let bytes =
            hex("5A-6B-17-05-9E-70-F1-19-2C-27-0C-46-6F-63-61-6C-20-42-61-74-68-79-73-C4-FF");
        let d = parse_inquiry_data(&Frame::from_report(&bytes).unwrap()).unwrap();
        assert_eq!(d.name, "Focal Bathys");
        assert_eq!(format_address(&d.address), "9E:70:F1:19:2C:27");
    }

    #[test]
    fn num_paired_and_feature() {
        let n = Frame::from_report(&hex("5A-6B-02-01-01-00")).unwrap();
        assert_eq!(parse_num_paired(&n), Some(1));
        let f = Frame::from_report(&hex("5A-6B-03-09-01-04-00")).unwrap();
        assert!(get_feature(feature::MAX_PAIRED).expect.matches(&f) == Some(Ok(())));
        assert_eq!(parse_feature_byte(&f), Some(4));
    }

    #[test]
    fn parses_current_codec_from_log() {
        let f = Frame::from_report(&hex("5A-6B-03-09-03-40-00")).unwrap();
        assert!(get_feature(feature::CODEC_CURRENT).expect.matches(&f) == Some(Ok(())));
        assert_eq!(
            parse_feature_value(&f),
            Some(codec::APTX_ADAPTIVE_HIGH_QUALITY)
        );
        let two_bytes = Frame::from_report(&hex("5A-6B-04-09-03-00-02")).unwrap();
        assert_eq!(parse_feature_value(&two_bytes), Some(codec::LC3));
    }

    #[test]
    fn auto_reconnect_frames_match_log() {
        let req = set_feature(feature::AUTO_RECONNECT, 1);
        assert_eq!(
            &req.frame.to_report()[..7],
            hex("03-5A-6B-03-0A-00-01").as_slice()
        );
        let ack = Frame::from_report(&hex("5A-02-0A-6B-00-0A-00-00-00-00-00-00-00")).unwrap();
        assert_eq!(req.expect.matches(&ack), Some(Ok(())));
        let value = Frame::from_report(&hex("5A-6B-03-09-00-01-00")).unwrap();
        assert!(get_feature(feature::AUTO_RECONNECT).expect.matches(&value) == Some(Ok(())));
        assert_eq!(parse_feature_byte(&value), Some(1));
    }
}
