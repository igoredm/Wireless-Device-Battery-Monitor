//! Device model and the driver chain (port of devices.h / devices.cpp / drivers.h).
use std::collections::BTreeSet;

use crate::drivers;
use crate::hid::{self, HidColl};
use crate::i18n::n_;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeviceKind {
    #[default]
    Other,
    Mouse,
    Headset,
    Keyboard,
    Gamepad,
    Phone,
    Microphone,
    /// In-ear headphones (a left and a right bud).
    Earbuds,
}

impl DeviceKind {
    /// Segoe MDL2 / Fluent icon glyph for the kind (same font the original draws with).
    pub fn glyph(self) -> char {
        match self {
            DeviceKind::Mouse => '\u{E962}',
            DeviceKind::Headset | DeviceKind::Earbuds => '\u{E7F6}',
            DeviceKind::Keyboard => '\u{E765}',
            DeviceKind::Gamepad => '\u{E7FC}',
            DeviceKind::Phone => '\u{E8EA}',
            DeviceKind::Microphone => '\u{E720}',
            DeviceKind::Other => '\u{E83F}',
        }
    }

    /// The kind's name in the current language.
    pub fn label(self) -> String {
        crate::i18n::tr(self.label_en())
    }

    pub fn label_en(self) -> &'static str {
        match self {
            DeviceKind::Mouse => n_("Mouse"),
            DeviceKind::Headset => n_("Headset"),
            DeviceKind::Keyboard => n_("Keyboard"),
            DeviceKind::Gamepad => n_("Controller"),
            DeviceKind::Phone => n_("Phone"),
            DeviceKind::Microphone => n_("Microphone"),
            DeviceKind::Earbuds => n_("Earbuds"),
            DeviceKind::Other => n_("Device"),
        }
    }
}

/// Levels of the two buds of earbuds (-1: that bud is not reported, e.g. in its case or off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buds {
    pub left: i32,
    pub right: i32,
    pub left_charging: bool,
    pub right_charging: bool,
    /// The charging case: level and whether it charges, when the device reports it.
    pub case: Option<(i32, bool)>,
}

impl Buds {
    /// Level and charging state of the bud with the lower level (None when neither reports).
    pub fn weaker(&self) -> Option<(i32, bool)> {
        [(self.left, self.left_charging), (self.right, self.right_charging)].into_iter().filter(|b| b.0 >= 0).min_by_key(|b| b.0)
    }

    /// Both buds' levels when both are reported ("12% 86%", left first), else None.
    pub fn both(&self) -> Option<String> {
        (self.left >= 0 && self.right >= 0).then(|| format!("{}% {}%", self.left, self.right))
    }

    /// "L 80%  R 75%" (a bud that is not reported: "L –").
    pub fn text(&self) -> String {
        let side = |name: &str, v: i32, c: bool| {
            if v < 0 { format!("{} –", crate::i18n::tr(name)) } else { format!("{} {v}%{}", crate::i18n::tr(name), if c { "⚡" } else { "" }) }
        };
        format!("{}  {}", side(n_("L"), self.left, self.left_charging), side(n_("R"), self.right, self.right_charging))
    }

    /// "L: 80%, R: 75%" for menus and tooltips (a bud that is not reported: "L: –"); None when
    /// neither bud reports.
    pub fn levels(&self) -> Option<String> {
        if self.left < 0 && self.right < 0 {
            return None;
        }
        let side = |name: &str, v: i32| if v < 0 { format!("{}: –", crate::i18n::tr(name)) } else { format!("{}: {v}%", crate::i18n::tr(name)) };
        Some(format!("{}, {}", side(n_("L"), self.left), side(n_("R"), self.right)))
    }
}

/// One battery-powered device, as seen during a single poll.
#[derive(Clone, Debug)]
pub struct DeviceStatus {
    /// Stable identity across polls.
    pub key: String,
    pub name: String,
    /// "2.4 GHz receiver", "Bluetooth", "USB cable", ...
    pub via: String,
    pub kind: DeviceKind,
    /// 0..=100, or -1 when unknown right now.
    pub percent: i32,
    /// Extra breakdown, e.g. "L 80%, R 75%, case 40%".
    pub detail: String,
    /// Earbuds that report each bud: their levels (`percent` is the lower one in use).
    pub buds: Option<Buds>,
    /// Set when the device only reports coarse levels ("Low", "Full", ...).
    pub level_text: String,
    /// Level of the spare battery in the device's charging dock; None = no dock, or it is empty.
    pub dock_percent: Option<i32>,
    pub charging: bool,
    /// On power and fully charged.
    pub full: bool,
    pub asleep: bool,
    /// false: receiver present, device switched off.
    pub connected: bool,
    pub millivolts: i32,
    /// Keep showing the last value this long if the device stops answering.
    pub grace_sec: u32,
    pub vid: u16,
    /// The USB / HID product (vid, pid) the device is read through - its receiver, cable or BT HID
    /// link - or None when it is not read over HID (Bluetooth properties, BLE broadcasts, XInput).
    /// When that hardware is gone, the device is gone too (not "asleep").
    pub hw: Option<(u16, u16)>,
}

impl Default for DeviceStatus {
    fn default() -> Self {
        Self {
            key: String::new(),
            name: String::new(),
            via: String::new(),
            kind: DeviceKind::Other,
            percent: -1,
            detail: String::new(),
            buds: None,
            level_text: String::new(),
            dock_percent: None,
            charging: false,
            full: false,
            asleep: false,
            connected: true,
            millivolts: 0,
            grace_sec: 0,
            vid: 0,
            hw: None,
        }
    }
}

impl DeviceStatus {
    /// Short status text, e.g. "93%", "45% ⚡", "Full ✓", "?".
    pub fn status_text(&self) -> String {
        if !self.connected {
            return crate::i18n::tr("disconnected");
        }
        let mut t = if self.percent >= 0 {
            format!("{}%", self.percent)
        } else if !self.level_text.is_empty() {
            self.level_text.clone()
        } else {
            "?".into()
        };
        if self.is_full() {
            t.push_str(" ✓");
        } else if self.charging {
            t.push_str(" ⚡");
        }
        if self.asleep {
            t.push_str(&format!(" ({})", crate::i18n::tr("asleep")));
        }
        t
    }

    /// On power and fully charged: reported as full, or charging at 100%.
    pub fn is_full(&self) -> bool {
        self.connected && (self.full || (self.charging && self.percent >= 100))
    }

    /// Charging (on power, not yet full).
    pub fn is_charging(&self) -> bool {
        self.connected && self.charging && !self.is_full()
    }

    /// Level text: "32%", a coarse level ("Low"), or "?".
    pub fn level(&self) -> String {
        if self.percent >= 0 {
            format!("{}%", self.percent)
        } else if !self.level_text.is_empty() {
            self.level_text.clone()
        } else {
            "?".into()
        }
    }

    /// State of the spare battery in the dock: "63%, charging" / "full".
    pub fn dock_text(&self) -> Option<String> {
        self.dock_percent.map(|p| if p >= 100 { crate::i18n::tr("full") } else { crate::i18n::trf("{level}%, charging", &[("level", &p.to_string())]) })
    }

    /// "32%, charging" / "100%, fully charged" / "32%, on battery" - for menus and tooltips.
    pub fn charge_text(&self) -> String {
        let state = if !self.connected {
            n_("disconnected")
        } else if self.is_full() {
            n_("fully charged")
        } else if self.charging {
            n_("charging")
        } else if self.asleep {
            n_("asleep, last known level")
        } else {
            n_("on battery")
        };
        let level = self.buds.filter(|_| self.percent >= 0).and_then(|b| b.levels()).unwrap_or_else(|| self.level());
        format!("{}, {}", level, crate::i18n::tr(state))
    }
}

/// Connection names drivers use, translated where they are shown.
pub const CONNECTIONS: [&str; 10] = [
    n_("2.4 GHz receiver"),
    n_("2.4 GHz dongle"),
    n_("USB cable"),
    n_("USB / Bluetooth"),
    n_("Logitech receiver"),
    n_("Razer receiver"),
    n_("Corsair receiver"),
    n_("USB base station"),
    n_("UWB base station"),
    n_("Wireless adapter"),
];

/// Shared state handed through the driver chain. A driver claims a (vid, pid) once it has handled
/// it, so the generic fallbacks further down do not report it twice.
#[derive(Default)]
pub struct PollContext {
    pub colls: Vec<HidColl>,
    pub claimed: BTreeSet<(u16, u16)>,
    /// Container ids the Bluetooth fallback already reported.
    pub bt_containers: BTreeSet<String>,
    pub out: Vec<DeviceStatus>,
}

impl PollContext {
    pub fn is_claimed(&self, vid: u16, pid: u16) -> bool {
        self.claimed.contains(&(vid, pid))
    }
    pub fn claim(&mut self, vid: u16, pid: u16) {
        self.claimed.insert((vid, pid));
    }
    pub fn add(&mut self, mut s: DeviceStatus, vid: u16, pid: u16) {
        if s.vid == 0 {
            s.vid = vid;
        }
        self.out.push(s);
        self.claim(vid, pid);
    }
    /// Distinct (vid, pid) pairs of one vendor that are not claimed yet.
    pub fn products(&self, vid: u16) -> Vec<(u16, u16)> {
        let set: BTreeSet<(u16, u16)> = self.colls.iter().filter(|c| c.vid == vid && !self.is_claimed(c.vid, c.pid)).map(|c| (c.vid, c.pid)).collect();
        set.into_iter().collect()
    }
    /// Collections of one product matching `pred` (cloned so drivers can mutate the context).
    pub fn find(&self, vid: u16, pid: u16, pred: impl Fn(&HidColl) -> bool) -> Vec<HidColl> {
        self.colls.iter().filter(|c| c.vid == vid && c.pid == pid && pred(c)).cloned().collect()
    }
}

/// HKCU\Software\WirelessBattery\IgnoreDevices: "VID:PID" pairs (same setting as the original).
fn ignored_devices() -> Vec<(u16, u16)> {
    let Some(text) = crate::util::reg_get_string("Software\\WirelessBattery", "IgnoreDevices") else {
        return Vec::new();
    };
    text.split([',', ';', ' '])
        .filter_map(|p| {
            let (v, d) = p.trim().split_once(':')?;
            Some((u16::from_str_radix(v, 16).ok()?, u16::from_str_radix(d, 16).ok()?))
        })
        .collect()
}

/// HID collections from the last enumeration. Enumerating opens every HID device on the system,
/// so a periodic battery read reuses this list; it is rebuilt on a rescan (device change, manual
/// "Refresh devices", start-up) and at least every RESCAN_AFTER.
static HID_CACHE: std::sync::Mutex<Option<HidCache>> = std::sync::Mutex::new(None);
const RESCAN_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);

struct HidCache {
    at: std::time::Instant,
    /// Interface paths present at the last enumeration (sorted).
    paths: Vec<String>,
    colls: Vec<HidColl>,
}

/// The HID collections to read. A cached list is reused only while the set of HID interfaces is
/// unchanged (a cheap SetupAPI listing, no device opened), so a device plugged in or removed is
/// picked up by the next read even if the device-change message was missed.
/// Devices no driver may send anything to. Logitech G435 (046D:0ACB): its dongle reports the
/// battery only from bootloader mode, which cuts the audio (issue #10); it has no HID++ channel and
/// no standard battery usage, so it is skipped outright rather than relying on that.
const NEVER_TOUCH: [(u16, u16); 2] = [(0x046D, 0x0ACB), (0x046D, 0x0ADF)];

fn hid_collections(rescan: bool) -> Vec<HidColl> {
    let mut cache = HID_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let paths = hid::interface_paths();
    if let Some(c) = cache.as_ref()
        && !rescan
        && c.at.elapsed() < RESCAN_AFTER
        && c.paths == paths
    {
        return c.colls.clone();
    }
    let colls: Vec<HidColl> = hid::enum_hid().into_iter().filter(|c| !NEVER_TOUCH.contains(&(c.vid, c.pid))).collect();
    *cache = Some(HidCache { at: std::time::Instant::now(), paths, colls: colls.clone() });
    crate::drivers::sdi::forget_containers();
    colls
}

/// Quick read of only the given receivers / cables (vid, pid): used to notice soon that a device
/// behind a plugged-in receiver was switched on or woke up, which Windows does not report.
/// Bluetooth, BLE and XInput are not touched.
pub fn poll_hardware(ids: &[(u16, u16)]) -> Vec<DeviceStatus> {
    let colls: Vec<HidColl> = hid_collections(false).into_iter().filter(|c| ids.contains(&(c.vid, c.pid))).collect();
    if colls.is_empty() {
        return Vec::new();
    }
    let mut ctx = PollContext { colls, ..Default::default() };
    drivers::steelseries::poll(&mut ctx);
    drivers::logitech_g321::poll(&mut ctx);
    drivers::logitech_prox2::poll(&mut ctx);
    drivers::logitech_g435::poll(&mut ctx);
    drivers::logitech_g735::poll(&mut ctx);
    drivers::logitech_g533::poll(&mut ctx);
    drivers::astro::poll(&mut ctx);
    drivers::astro_a50_gen4::poll(&mut ctx);
    drivers::logitech::poll(&mut ctx);
    drivers::blackshark_v3x::poll(&mut ctx);
    drivers::kraken_v3pro::poll(&mut ctx);
    drivers::razer::poll(&mut ctx);
    drivers::roccat::poll(&mut ctx);
    drivers::fnatic::poll(&mut ctx);
    drivers::attackshark::poll(&mut ctx);
    drivers::blackshark::poll(&mut ctx);
    drivers::wlmouse::poll(&mut ctx);
    drivers::hyperx::poll(&mut ctx);
    drivers::blackshark_v2pro::poll(&mut ctx);
    drivers::corsair::poll(&mut ctx);
    drivers::corsair_bragi::poll(&mut ctx);
    drivers::aula::poll(&mut ctx);
    drivers::atk::poll(&mut ctx);
    drivers::atk_serial::poll(&mut ctx);
    drivers::mchose::poll(&mut ctx);
    drivers::evision::poll(&mut ctx);
    drivers::darmoshark::poll(&mut ctx);
    drivers::sonix::poll(&mut ctx);
    drivers::compx::poll(&mut ctx);
    drivers::compx_extra::poll(&mut ctx);
    drivers::aula_mouse::poll(&mut ctx);
    drivers::aula_infortech::poll(&mut ctx); // Pitwall: AULA "Infortech" receivers 38A6
    drivers::mchose_modern::poll(&mut ctx);
    drivers::ajazz_aj139::poll(&mut ctx);
    drivers::hyperx_core::poll(&mut ctx);
    drivers::areson::poll(&mut ctx);
    drivers::razer_nari::poll(&mut ctx);
    drivers::rog_pelta::poll(&mut ctx);
    drivers::atk_mercury::poll(&mut ctx);
    drivers::rongyuan::poll(&mut ctx);
    drivers::mchose_v9::poll(&mut ctx);
    drivers::barracuda_chroma::poll(&mut ctx);
    drivers::barracuda_x::poll(&mut ctx);
    drivers::hyperx_cloud2::poll(&mut ctx);
    drivers::rawm::poll(&mut ctx);
    drivers::jabra::poll(&mut ctx);
    drivers::zowie::poll(&mut ctx);
    drivers::nzxt::poll(&mut ctx);
    drivers::keychron::poll(&mut ctx);
    drivers::keychron_nape::poll(&mut ctx);
    drivers::via_dongle::poll(&mut ctx);
    drivers::hyperx_flight::poll(&mut ctx); // tested on hardware (PR #93); before hyperx_legacy
    drivers::hyperx_legacy::poll(&mut ctx);
    drivers::drivercore::poll(&mut ctx);
    drivers::flydigi::poll(&mut ctx);
    drivers::gamesir::poll(&mut ctx);
    drivers::sinowealth::poll(&mut ctx);
    drivers::endgame_4k::poll(&mut ctx);
    drivers::mchose_glw::poll(&mut ctx);
    drivers::mchose_g3::poll(&mut ctx);
    drivers::ajazz_aj179::poll(&mut ctx);
    drivers::aula_hub::poll(&mut ctx);
    drivers::mchose_x9::poll(&mut ctx);
    drivers::mchose_v9pro2::poll(&mut ctx);
    drivers::piao::poll(&mut ctx);
    drivers::dareu::poll(&mut ctx);
    drivers::dareu_em901x::poll(&mut ctx);
    drivers::niceboy::poll(&mut ctx);
    drivers::raigor::poll(&mut ctx);
    drivers::fractal::poll(&mut ctx);
    drivers::audeze::poll(&mut ctx);
    drivers::incott::poll(&mut ctx);
    drivers::asus_mouse::poll(&mut ctx);
    drivers::machenike::poll(&mut ctx);
    drivers::playstation::poll(&mut ctx);
    drivers::generic::poll(&mut ctx);
    ctx.out
}

/// What to do with a device that was shown before but is missing from the latest read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// Gone: unplugged receiver / cable, or a Bluetooth / BLE / XInput device that disconnected.
    Remove,
    /// Its receiver is still there and the device sleeps: keep the last value, dimmed.
    Asleep,
    /// Its hardware is still there and it missed one answer: keep the last value as it was.
    Keep,
}

/// Decides about a device missing from a read. `hw` is the hardware it is read through and
/// `hw_present` whether that hardware is still plugged in; `age_sec` is the time since it last
/// answered. Bluetooth / BLE / XInput devices (no `hw`) are only reported while connected, so a
/// missing one has disconnected.
pub fn missing_device(hw: Option<(u16, u16)>, hw_present: bool, grace_sec: u32, age_sec: u64, min_grace_sec: u64) -> Missing {
    if hw.is_none() || !hw_present {
        return Missing::Remove;
    }
    if grace_sec > 0 && age_sec < grace_sec as u64 {
        Missing::Asleep
    } else if age_sec < min_grace_sec {
        Missing::Keep
    } else {
        Missing::Remove
    }
}

/// Whether a HID product (vid, pid) was present in the latest device enumeration.
pub fn hid_present(id: (u16, u16)) -> bool {
    let cache = HID_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.as_ref().is_some_and(|c| c.colls.iter().any(|c| (c.vid, c.pid) == id))
}

/// Queries every supported device, enumerating them first. Blocking (a second or two); call from a
/// worker thread.
pub fn poll_all_devices() -> Vec<DeviceStatus> {
    poll_devices(true)
}

/// Queries every supported device. `rescan` = enumerate the HID devices again; otherwise the list
/// from the last enumeration is reused (only the battery levels are read).
pub fn poll_devices(rescan: bool) -> Vec<DeviceStatus> {
    in_mta(|| poll_with(hid_collections(rescan)))
}

/// Reads only the AirPods / Beats (their broadcasts): the quick read uses it so taking a bud out
/// of the case shows within seconds instead of at the next full read.
pub fn poll_airpods() -> Vec<DeviceStatus> {
    in_mta(|| {
        let mut ctx = PollContext::default();
        drivers::airpods::poll(&mut ctx);
        ctx.out
    })
}

/// Runs `f` on this thread joined to the MTA, then stops the broadcast listening.
fn in_mta(f: impl FnOnce() -> Vec<DeviceStatus>) -> Vec<DeviceStatus> {
    // WinRT (BLE watcher, Windows.Gaming.Input) needs the poll thread in the MTA. The shared BLE
    // watcher outlives each poll thread, so the MTA is kept alive once for the whole process
    // (CoIncrementMTAUsage); each thread then joins it and leaves it again (balanced).
    static MTA: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    MTA.get_or_init(|| unsafe {
        // The cookie is deliberately never released: the MTA lives as long as the process.
        let _ = windows::Win32::System::Com::CoIncrementMTAUsage();
    });
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { windows::Win32::System::WinRT::RoUninitialize() };
            }
        }
    }
    let _apartment = Apartment(unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) }.is_ok());
    let out = f();
    // Broadcast listening is only needed while reading; between reads it would keep waking the
    // process for every Bluetooth LE advertisement nearby.
    crate::ble::stop_watcher();
    out
}

fn poll_with(colls: Vec<HidColl>) -> Vec<DeviceStatus> {
    let mut ctx = PollContext { colls, ..Default::default() };
    let ignored = ignored_devices();
    if !ignored.is_empty() {
        ctx.colls.retain(|c| !ignored.contains(&(c.vid, c.pid)));
    }
    // Vendor drivers first, generic fallbacks last (same order as the original chain).
    drivers::steelseries::poll(&mut ctx);
    drivers::logitech_g321::poll(&mut ctx);
    drivers::logitech_prox2::poll(&mut ctx);
    drivers::logitech_g435::poll(&mut ctx);
    drivers::logitech_g735::poll(&mut ctx);
    drivers::logitech_g533::poll(&mut ctx);
    drivers::astro::poll(&mut ctx);
    drivers::astro_a50_gen4::poll(&mut ctx);
    drivers::logitech::poll(&mut ctx);
    drivers::blackshark_v3x::poll(&mut ctx);
    drivers::kraken_v3pro::poll(&mut ctx);
    drivers::razer::poll(&mut ctx);
    drivers::roccat::poll(&mut ctx);
    drivers::fnatic::poll(&mut ctx);
    drivers::attackshark::poll(&mut ctx);
    drivers::blackshark::poll(&mut ctx);
    drivers::wlmouse::poll(&mut ctx);
    drivers::hyperx::poll(&mut ctx);
    drivers::blackshark_v2pro::poll(&mut ctx);
    drivers::corsair::poll(&mut ctx);
    drivers::corsair_bragi::poll(&mut ctx);
    drivers::aula::poll(&mut ctx);
    drivers::atk::poll(&mut ctx);
    drivers::atk_serial::poll(&mut ctx);
    drivers::mchose::poll(&mut ctx);
    drivers::evision::poll(&mut ctx);
    drivers::darmoshark::poll(&mut ctx);
    drivers::sonix::poll(&mut ctx);
    drivers::compx::poll(&mut ctx);
    drivers::compx_extra::poll(&mut ctx);
    drivers::aula_mouse::poll(&mut ctx);
    drivers::aula_infortech::poll(&mut ctx); // Pitwall: AULA "Infortech" receivers 38A6
    drivers::mchose_modern::poll(&mut ctx);
    drivers::ajazz_aj139::poll(&mut ctx);
    drivers::hyperx_core::poll(&mut ctx);
    drivers::areson::poll(&mut ctx);
    drivers::razer_nari::poll(&mut ctx);
    drivers::rog_pelta::poll(&mut ctx);
    drivers::atk_mercury::poll(&mut ctx);
    drivers::rongyuan::poll(&mut ctx);
    drivers::mchose_v9::poll(&mut ctx);
    drivers::barracuda_chroma::poll(&mut ctx);
    drivers::barracuda_x::poll(&mut ctx);
    drivers::hyperx_cloud2::poll(&mut ctx);
    drivers::rawm::poll(&mut ctx);
    drivers::jabra::poll(&mut ctx);
    drivers::zowie::poll(&mut ctx);
    drivers::nzxt::poll(&mut ctx);
    drivers::keychron::poll(&mut ctx);
    drivers::keychron_nape::poll(&mut ctx);
    drivers::via_dongle::poll(&mut ctx);
    drivers::hyperx_flight::poll(&mut ctx); // tested on hardware (PR #93); before hyperx_legacy
    drivers::hyperx_legacy::poll(&mut ctx);
    drivers::drivercore::poll(&mut ctx);
    drivers::flydigi::poll(&mut ctx);
    drivers::gamesir::poll(&mut ctx);
    drivers::sinowealth::poll(&mut ctx);
    drivers::endgame_4k::poll(&mut ctx);
    drivers::mchose_glw::poll(&mut ctx);
    drivers::mchose_g3::poll(&mut ctx);
    drivers::ajazz_aj179::poll(&mut ctx);
    drivers::aula_hub::poll(&mut ctx);
    drivers::mchose_x9::poll(&mut ctx);
    drivers::mchose_v9pro2::poll(&mut ctx);
    drivers::piao::poll(&mut ctx);
    drivers::dareu::poll(&mut ctx);
    drivers::dareu_em901x::poll(&mut ctx);
    drivers::niceboy::poll(&mut ctx);
    drivers::raigor::poll(&mut ctx);
    drivers::fractal::poll(&mut ctx);
    drivers::audeze::poll(&mut ctx);
    drivers::incott::poll(&mut ctx);
    drivers::asus_mouse::poll(&mut ctx);
    drivers::machenike::poll(&mut ctx);
    drivers::playstation::poll(&mut ctx);
    drivers::airpods::poll(&mut ctx);
    drivers::bluetooth::poll(&mut ctx);
    drivers::generic::poll(&mut ctx);
    drivers::xinput::poll(&mut ctx);
    ctx.out
}

/// Prints every HID collection (to help adding support for new devices).
pub fn dump_hid_collections() -> String {
    let mut out = String::from("VID:PID    IF  UsagePg:Usage  In  Out Feat  Product\n");
    for c in hid::enum_hid() {
        out += &format!(
            "{:04X}:{:04X}  {:2}  {:04X}:{:04X}    {:3} {:3} {:3}   {}\n",
            c.vid,
            c.pid,
            c.iface,
            c.page(),
            c.usage(),
            c.in_len(),
            c.out_len(),
            c.feat_len(),
            hid::product_string(&c)
        );
    }
    out
}

#[cfg(test)]
mod missing_tests {
    use super::*;
    const RX: Option<(u16, u16)> = Some((0x046D, 0xC543));

    #[test]
    fn weaker_bud_is_shown() {
        let b = Buds { left: 80, right: 40, left_charging: false, right_charging: true, case: None };
        assert_eq!(b.weaker(), Some((40, true)));
        assert_eq!(Buds { left: -1, right: 70, ..Default::default() }.weaker(), Some((70, false)));
        assert_eq!(Buds { left: -1, right: -1, ..Default::default() }.weaker(), None);
    }

    #[test]
    fn unplugged_receiver_removes_at_once() {
        assert_eq!(missing_device(RX, false, 1800, 5, 150), Missing::Remove);
    }

    #[test]
    fn sleeping_mouse_on_a_plugged_receiver_is_asleep() {
        assert_eq!(missing_device(RX, true, 1800, 120, 150), Missing::Asleep);
        assert_eq!(missing_device(RX, true, 1800, 1801, 150), Missing::Remove);
    }

    #[test]
    fn one_missed_answer_keeps_the_value_briefly() {
        assert_eq!(missing_device(RX, true, 0, 60, 150), Missing::Keep);
        assert_eq!(missing_device(RX, true, 0, 151, 150), Missing::Remove);
    }

    #[test]
    fn bluetooth_devices_disappear_when_disconnected() {
        assert_eq!(missing_device(None, false, 300, 1, 150), Missing::Remove);
    }
}
