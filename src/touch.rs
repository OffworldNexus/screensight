//! Direct evdev touchscreen reader.
//!
//! GPUI 0.2.2 has no Wayland `wl_touch` support, and under cage the touch panel
//! produces no pointer events. Rather than depend on the compositor, we read
//! the touchscreen input device ourselves (as Kodi does) and forward normalized
//! `[0, 1]` coordinates to the UI. Works with the `ft5x06` controller on the
//! Raspberry Pi Touch Display.
//!
//! Requires read access to `/dev/input/event*` (root, or the `input` group).

use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A touch point, normalized to the display's `[0, 1]` range.
#[derive(Clone, Copy, Debug)]
pub struct TouchSample {
    pub fx: f32,
    pub fy: f32,
}

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const BTN_TOUCH: u16 = 0x14a;

const IOC_READ: u64 = 2;
const IOC_TYPESHIFT: u64 = 8;
const IOC_SIZESHIFT: u64 = 16;
const IOC_DIRSHIFT: u64 = 30;

const fn ioc(dir: u64, ty: u64, nr: u64, size: u64) -> libc::c_ulong {
    ((dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr) | (size << IOC_SIZESHIFT)) as libc::c_ulong
}

#[repr(C)]
struct InputEvent {
    sec: i64,
    usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct InputAbsinfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

fn absinfo(fd: libc::c_int, code: u16) -> Option<InputAbsinfo> {
    let mut info = InputAbsinfo::default();
    let req = ioc(IOC_READ, b'E' as u64, 0x40 + code as u64, 24);
    let rc = unsafe { libc::ioctl(fd, req, &mut info as *mut _) };
    (rc == 0 && info.maximum > info.minimum).then_some(info)
}

fn device_name(fd: libc::c_int) -> String {
    let mut buf = [0u8; 256];
    let req = ioc(IOC_READ, b'E' as u64, 0x06, buf.len() as u64);
    let rc = unsafe { libc::ioctl(fd, req, buf.as_mut_ptr()) };
    if rc < 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Spawn a background thread that streams touches into `sink`.
pub fn spawn_reader(sink: Arc<Mutex<Vec<TouchSample>>>) {
    std::thread::Builder::new()
        .name("evdev-touch".into())
        .spawn(move || {
            if let Err(err) = run(sink) {
                log::warn!("touch reader stopped: {err}");
            }
        })
        .expect("spawn touch reader");
}

fn run(sink: Arc<Mutex<Vec<TouchSample>>>) -> anyhow::Result<()> {
    let (file, x_info, y_info, name) = find_touchscreen()
        .ok_or_else(|| anyhow::anyhow!("no touchscreen evdev device found"))?;
    log::info!(
        "touch reader using {name:?} x=[{}..{}] y=[{}..{}]",
        x_info.minimum,
        x_info.maximum,
        y_info.minimum,
        y_info.maximum
    );

    let fd = file.as_raw_fd();
    let mut ev = std::mem::MaybeUninit::<InputEvent>::uninit();
    let mut x = x_info.minimum;
    let mut y = y_info.minimum;
    let mut touching = false;
    let mut was_touching = false;
    let mut last_emit = Instant::now();

    loop {
        let n = unsafe {
            libc::read(
                fd,
                ev.as_mut_ptr() as *mut libc::c_void,
                std::mem::size_of::<InputEvent>(),
            )
        };
        if n < std::mem::size_of::<InputEvent>() as isize {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err.into());
        }
        let ev = unsafe { ev.assume_init_ref() };
        match (ev.type_, ev.code) {
            (EV_ABS, ABS_MT_POSITION_X) | (EV_ABS, ABS_X) => x = ev.value,
            (EV_ABS, ABS_MT_POSITION_Y) | (EV_ABS, ABS_Y) => y = ev.value,
            (EV_ABS, ABS_MT_TRACKING_ID) => touching = ev.value >= 0,
            (EV_KEY, BTN_TOUCH) => touching = ev.value != 0,
            (EV_SYN, SYN_REPORT) => {
                // Emit immediately on touch-down, then throttle drag samples so
                // a slide produces a pleasant trail instead of a flood.
                if touching && (!was_touching || last_emit.elapsed() >= Duration::from_millis(40)) {
                    let fx =
                        (x - x_info.minimum) as f32 / (x_info.maximum - x_info.minimum) as f32;
                    let fy =
                        (y - y_info.minimum) as f32 / (y_info.maximum - y_info.minimum) as f32;
                    if let Ok(mut q) = sink.lock() {
                        if q.len() > 64 {
                            q.clear();
                        }
                        q.push(TouchSample {
                            fx: fx.clamp(0.0, 1.0),
                            fy: fy.clamp(0.0, 1.0),
                        });
                    }
                    last_emit = Instant::now();
                }
                was_touching = touching;
            }
            _ => {}
        }
    }
}

fn find_touchscreen() -> Option<(File, InputAbsinfo, InputAbsinfo, String)> {
    let mut fallback = None;
    for i in 0..32 {
        let Ok(file) = File::open(format!("/dev/input/event{i}")) else {
            continue;
        };
        let fd = file.as_raw_fd();
        let name = device_name(fd);
        let x = absinfo(fd, ABS_MT_POSITION_X).or_else(|| absinfo(fd, ABS_X));
        let y = absinfo(fd, ABS_MT_POSITION_Y).or_else(|| absinfo(fd, ABS_Y));
        let (Some(x), Some(y)) = (x, y) else {
            continue;
        };
        if name.to_ascii_lowercase().contains("ft5x06") {
            return Some((file, x, y, name));
        }
        if fallback.is_none() {
            fallback = Some((file, x, y, name));
        }
    }
    fallback
}
