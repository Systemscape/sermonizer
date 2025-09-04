use std::fmt;
use std::time::{Duration, SystemTime};

pub struct CachedTimestamp {
    last_update: u128,
    cached_string: String,
}

impl CachedTimestamp {
    pub fn new() -> Self {
        Self {
            last_update: 0,
            cached_string: String::new(),
        }
    }

    pub fn now_rfc3339(&mut self) -> &str {
        let current_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis();

        // Only update if more than 1ms has passed (reduces allocations)
        if current_ms != self.last_update {
            self.cached_string = format!("{}", Timestamp(current_ms));
            self.last_update = current_ms;
        }

        &self.cached_string
    }
}

struct Timestamp(u128);

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = self.0;
        let secs = (ms / 1000) as i64;
        let millis = (ms % 1000) as u32;
        let tm = secs_to_ymdhms_utc(secs);
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            tm.0, tm.1, tm.2, tm.3, tm.4, tm.5, millis
        )
    }
}

// Optimized UTC time conversion
fn secs_to_ymdhms_utc(s: i64) -> (i32, u32, u32, u32, u32, u32) {
    const SECS_PER_DAY: i64 = 86_400;
    let z = s.div_euclid(SECS_PER_DAY);
    let secs_of_day = s.rem_euclid(SECS_PER_DAY);
    let a = z + 719468;
    let era = if a >= 0 { a } else { a - 146096 };
    let doe = a - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i32) + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + (m <= 2) as i32;
    let hour = (secs_of_day / 3600) as u32;
    let min = ((secs_of_day % 3600) / 60) as u32;
    let sec = (secs_of_day % 60) as u32;
    (y, m as u32, d as u32, hour, min, sec)
}