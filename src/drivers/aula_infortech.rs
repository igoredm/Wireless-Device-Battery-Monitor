//! AULA mice on the "AULA Receiver" (Infortech SDK): Kunlun X5 Ultra / MAX / mini, L9, L5 Ultra,
//! SC880 Ultra / MAX, SC900 V2 Ultra / Master / Pro, V9 Ultra(+), X9 Ultra, V3 MAX. Receivers
//! 38A6:28F9 and 38A6:2F1A.
//!
//! Written for Pitwall (github.com/igoredm/pitwall) from the official AULA HUB web driver
//! (hub.aulacn.com, bundled "InfortechDevice" SDK) and verified on a Kunlun X5 Ultra (mouse PID
//! 2F08) through the 38A6:28F9 receiver:
//!  * vendor collection FFA8:0002, 64-byte reports. Requests go out as output report 0x0B:
//!    `0B cmd len args...`, zero padded. Replies come back as input report 0x0A:
//!    `0A cmd len status data...`, status 0 = OK. Only read commands are sent:
//!      D0      IsConnectMouse        data[0] = 1 when the mouse is linked to the receiver
//!      A0 00   DevGetMouseConfigExt  data[0..1] mouse VID, data[2..3] mouse PID (LE) -> model name
//!      81 00   DevGetInfo            data[13] charging, data[14] battery percent, data[15] full
//!  * The receiver also pushes `0A 80 .. 04 level full charging` on battery changes; those are
//!    skipped here.
//! Write commands (0x84, 0x86, 0x88, ...) and the firmware ones (0xC1..0xC3) exist in the SDK and
//! are never sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x38A6;
const PIDS: [u16; 2] = [0x28F9, 0x2F1A];
const OUT: u8 = 0x0B;
const IN: u8 = 0x0A;
const IS_CONNECTED: u8 = 0xD0;
const MOUSE_CONFIG: u8 = 0xA0;
const INFO: u8 = 0x81;

fn model_name(mouse_pid: u16) -> &'static str {
    match mouse_pid {
        0x28F7 => "AULA L9",
        0x28FF => "AULA SC880 Ultra",
        0x28E7 => "AULA SC880 MAX",
        0x28E8 => "AULA X5 MAX",
        0x2EF4 => "AULA SC900 V2 Ultra",
        0x2EF6 => "AULA SC900 V2 Master",
        0x2EFE => "AULA SC900 V2 Pro",
        0x2F08 => "AULA X5 Ultra",
        0x2F10 => "AULA X5 mini",
        0x2F0C => "AULA L5 Ultra",
        0x2EEE => "AULA V9 Ultra+",
        0x2EF2 => "AULA V9 Ultra",
        0x2F14 => "AULA X9 Ultra",
        0x2F18 => "AULA V3 MAX",
        _ => "AULA wireless mouse",
    }
}

/// Payload of an OK reply to `cmd` (the bytes after `0A cmd len status`); None for anything else,
/// including the receiver's 0x80 pushes and error replies.
pub fn reply(r: &[u8], cmd: u8) -> Option<&[u8]> {
    let o = if r.first() == Some(&IN) { 1 } else { 0 };
    let u = r.get(o..)?;
    (u.len() > 3 && u[0] == cmd && u[2] == 0).then(|| &u[3..])
}

/// Sends one read command and waits for its reply payload.
fn ask(d: &HidDev, req: &[u8]) -> Option<Vec<u8>> {
    let cmd = req[1];
    d.flush();
    if !d.write(req) {
        return None;
    }
    let end = Deadline::in_ms(800);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms()) {
            if let Some(p) = reply(&r, cmd) {
                return Some(p.to_vec());
            }
        }
    }
    None
}

/// (percent, charging) from a DevGetInfo payload; percent None when out of range.
pub fn battery(p: &[u8]) -> Option<(Option<i32>, bool)> {
    if p.len() <= 15 {
        return None;
    }
    let level = p[14] as i32;
    Some(((0..=100).contains(&level).then_some(level), p[13] != 0 && p[15] == 0))
}

fn read(c: &HidColl, pid: u16) -> Option<DeviceStatus> {
    let d = HidDev::open(c)?;
    let linked = ask(&d, &[OUT, IS_CONNECTED])?; // no answer: not this protocol
    let base = DeviceStatus {
        key: format!("aula-infortech:{}:{}", hex4(VID), hex4(pid)),
        name: "AULA wireless mouse".into(),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, pid)),
        grace_sec: 1800,
        vid: VID,
        ..Default::default()
    };
    if linked.first() != Some(&1) {
        // The receiver answers but the mouse is off or out of range.
        return Some(DeviceStatus { connected: false, ..base });
    }
    let mouse_pid = ask(&d, &[OUT, MOUSE_CONFIG, 0x00]).filter(|p| p.len() > 3).map(|p| u16::from_le_bytes([p[2], p[3]])).unwrap_or(0);
    let s = DeviceStatus { name: model_name(mouse_pid).into(), ..base };
    Some(match ask(&d, &[OUT, INFO, 0x00]).as_deref().and_then(battery) {
        Some((percent, charging)) => DeviceStatus { percent: percent.unwrap_or(-1), charging, ..s },
        // Linked but did not answer: keep the last value.
        None => DeviceStatus { asleep: true, ..s },
    })
}

pub fn poll(ctx: &mut PollContext) {
    for pid in PIDS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page() == 0xFFA8 && c.usage() == 0x0002 && c.out_len() >= 4);
        for c in &colls {
            if let Some(s) = read(c, pid) {
                ctx.add(s, VID, pid);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(bytes: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 64];
        f[..bytes.len()].copy_from_slice(bytes);
        f
    }

    #[test]
    fn replies() {
        assert_eq!(reply(&frame(&[0x0A, 0xD0, 0x01, 0x00, 0x01]), IS_CONNECTED).map(|p| p[0]), Some(1));
        // without the report id (some stacks strip it)
        assert_eq!(reply(&frame(&[0xD0, 0x01, 0x00, 0x01]), IS_CONNECTED).map(|p| p[0]), Some(1));
        // error status, other command, battery push
        assert_eq!(reply(&frame(&[0x0A, 0xD0, 0x01, 0x05, 0x01]), IS_CONNECTED), None);
        assert_eq!(reply(&frame(&[0x0A, 0x81, 0x10, 0x00]), IS_CONNECTED), None);
        assert_eq!(reply(&frame(&[0x0A, 0x80, 0x04, 0x04, 0x50]), INFO), None);
        assert_eq!(reply(&[0x0A, 0xD0], IS_CONNECTED), None);
    }

    #[test]
    fn info() {
        let mut p = vec![0u8; 20];
        p[14] = 77;
        assert_eq!(battery(&p), Some((Some(77), false)));
        p[13] = 1;
        assert_eq!(battery(&p), Some((Some(77), true)));
        p[15] = 1; // full: on the cable but no longer charging
        assert_eq!(battery(&p), Some((Some(77), false)));
        p[14] = 200;
        assert_eq!(battery(&p), Some((None, false)));
        assert_eq!(battery(&p[..15]), None);
    }

    #[test]
    fn models() {
        assert_eq!(model_name(0x2F08), "AULA X5 Ultra");
        assert_eq!(model_name(0x1234), "AULA wireless mouse");
    }
}
