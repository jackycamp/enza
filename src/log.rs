use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const MAX_EVENTS: usize = 128;

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub fields: Vec<(String, String)>,
}

struct Store {
    events: VecDeque<Event>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

pub struct Span {
    name: &'static str,
    start: Instant,
    fields: Vec<(String, String)>,
}

pub fn timer(name: &'static str) -> Span {
    Span {
        name,
        start: Instant::now(),
        fields: Vec::new(),
    }
}

pub fn add_event(name: &str, fields: &[(&str, String)]) {
    let event = Event {
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect(),
    };

    let store = STORE.get_or_init(|| {
        Mutex::new(Store {
            events: VecDeque::with_capacity(MAX_EVENTS),
        })
    });

    if let Ok(mut store) = store.lock() {
        if store.events.len() == MAX_EVENTS {
            store.events.pop_front();
        }
        store.events.push_back(event);
    }
}

pub fn recent_events(limit: usize) -> Vec<Event> {
    let Some(store) = STORE.get() else {
        return Vec::new();
    };

    let Ok(store) = store.lock() else {
        return Vec::new();
    };

    store
        .events
        .iter()
        .rev()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
}

impl Span {
    pub fn field(&mut self, key: &str, value: impl ToString) {
        self.fields.push((key.to_string(), value.to_string()));
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let mut fields = Vec::with_capacity(self.fields.len() + 2);
        fields.push(("elapsed_ms", self.start.elapsed().as_millis().to_string()));
        if let Some(rss_mb) = current_rss_mb() {
            fields.push(("rss_mb", rss_mb));
        }
        for (key, value) in &self.fields {
            fields.push((key.as_str(), value.clone()));
        }
        add_event(self.name, &fields);
    }
}

pub fn current_rss_mb() -> Option<String> {
    current_rss_bytes().map(|bytes| format!("{:.1}", bytes as f64 / (1024.0 * 1024.0)))
}

#[cfg(target_os = "macos")]
type Integer = i32;
#[cfg(target_os = "macos")]
type Natural = u32;
#[cfg(target_os = "macos")]
type MachPort = u32;
#[cfg(target_os = "macos")]
type KernReturn = i32;
#[cfg(target_os = "macos")]
type MachMsgTypeNumber = u32;
#[cfg(target_os = "macos")]
type Policy = i32;
#[cfg(target_os = "macos")]
type MachVmSize = u64;

#[cfg(target_os = "macos")]
const KERN_SUCCESS: KernReturn = 0;
#[cfg(target_os = "macos")]
const MACH_TASK_BASIC_INFO: i32 = 20;

#[cfg(target_os = "macos")]
#[repr(C)]
struct TimeValue {
    seconds: Integer,
    microseconds: Integer,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct MachTaskBasicInfo {
    virtual_size: MachVmSize,
    resident_size: MachVmSize,
    resident_size_max: MachVmSize,
    user_time: TimeValue,
    system_time: TimeValue,
    policy: Policy,
    suspend_count: Integer,
}

#[cfg(target_os = "macos")]
fn current_rss_bytes() -> Option<u64> {
    unsafe extern "C" {
        fn mach_task_self() -> MachPort;
        fn task_info(
            target_task: MachPort,
            flavor: i32,
            task_info_out: *mut Integer,
            task_info_out_count: *mut MachMsgTypeNumber,
        ) -> KernReturn;
    }

    let mut info = MachTaskBasicInfo {
        virtual_size: 0,
        resident_size: 0,
        resident_size_max: 0,
        user_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        system_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        policy: 0,
        suspend_count: 0,
    };
    let mut count = (std::mem::size_of::<MachTaskBasicInfo>() / std::mem::size_of::<Natural>())
        as MachMsgTypeNumber;

    let result = unsafe {
        task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            (&mut info as *mut MachTaskBasicInfo).cast::<Integer>(),
            &mut count,
        )
    };

    if result == KERN_SUCCESS {
        Some(info.resident_size)
    } else {
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use std::mem::{offset_of, size_of};

    use super::{MachTaskBasicInfo, current_rss_bytes};

    #[test]
    fn mach_task_basic_info_layout_matches_resident_size_offset() {
        assert_eq!(offset_of!(MachTaskBasicInfo, resident_size), 8);
        assert_eq!(size_of::<MachTaskBasicInfo>(), 48);
    }

    #[test]
    fn macos_rss_query_returns_a_nonzero_value() {
        assert!(current_rss_bytes().is_some_and(|bytes| bytes > 0));
    }
}

#[cfg(not(target_os = "macos"))]
fn current_rss_bytes() -> Option<u64> {
    None
}
