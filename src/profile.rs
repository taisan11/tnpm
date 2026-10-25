//! Opt-in timings on stderr. Normal CLI output and behavior are unchanged.
use crate::json::Value;
use std::time::Instant;

pub struct Profile {
    section: &'static str,
    started: Option<Instant>,
    previous: Option<Instant>,
    values: Value,
}

impl Profile {
    pub fn new(section: &'static str) -> Self {
        let started = (std::env::var("TNPM_PROFILE").as_deref() == Ok("1")).then(Instant::now);
        Self {
            section,
            started,
            previous: started,
            values: Value::object(),
        }
    }

    pub fn mark(&mut self, name: &str) {
        if let Some(previous) = self.previous {
            let now = Instant::now();
            self.values.set(
                name,
                Value::Number(format!(
                    "{:.6}",
                    now.duration_since(previous).as_secs_f64() * 1000.0
                )),
            );
            self.previous = Some(now);
        }
    }

    pub fn text(&mut self, name: &str, value: impl Into<String>) {
        if self.started.is_some() {
            self.values.set(name, Value::String(value.into()));
        }
    }

    pub fn number(&mut self, name: &str, value: u64) {
        if self.started.is_some() {
            self.values.set(name, Value::Number(value.to_string()));
        }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            self.values.set("section", Value::from(self.section));
            self.values.set(
                "total_ms",
                Value::Number(format!("{:.6}", started.elapsed().as_secs_f64() * 1000.0)),
            );
            crate::progress::message(format_args!("tnpm-profile: {}", nojson::Json(&self.values)));
        }
    }
}
