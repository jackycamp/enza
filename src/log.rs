use std::collections::VecDeque;
use std::mem::size_of;
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
fn current_rss_bytes() -> Option<u64> {
    type Integer = i32;
    type Natural = u32;
    type MachPort = u32;
    type KernReturn = i32;
    type MachMsgTypeNumber = u32;
    type Policy = i32;

    const KERN_SUCCESS: KernReturn = 0;
    const TASK_BASIC_INFO: i32 = 20;

    #[repr(C)]
    struct TimeValue {
        seconds: Integer,
        microseconds: Integer,
    }

    #[repr(C)]
    struct TaskBasicInfo {
        suspend_count: Integer,
        virtual_size: usize,
        resident_size: usize,
        user_time: TimeValue,
        system_time: TimeValue,
        policy: Policy,
    }

    unsafe extern "C" {
        fn mach_task_self() -> MachPort;
        fn task_info(
            target_task: MachPort,
            flavor: i32,
            task_info_out: *mut Integer,
            task_info_out_count: *mut MachMsgTypeNumber,
        ) -> KernReturn;
    }

    let mut info = TaskBasicInfo {
        suspend_count: 0,
        virtual_size: 0,
        resident_size: 0,
        user_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        system_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        policy: 0,
    };
    let mut count = (size_of::<TaskBasicInfo>() / size_of::<Natural>()) as MachMsgTypeNumber;

    let result = unsafe {
        task_info(
            mach_task_self(),
            TASK_BASIC_INFO,
            (&mut info as *mut TaskBasicInfo).cast::<Integer>(),
            &mut count,
        )
    };

    if result == KERN_SUCCESS {
        Some(info.resident_size as u64)
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
fn current_rss_bytes() -> Option<u64> {
    None
}
