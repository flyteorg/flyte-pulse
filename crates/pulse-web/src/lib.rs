//! WebAssembly entry point: load a snapshot and answer dashboard API URLs.

use std::io::Read;

use flate2::read::GzDecoder;
use pulse_core::snapshot::Snapshot;
use pulse_core::time::{day_of, parse_date};
use pulse_core::views::Dataset;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Pulse {
    dataset: Dataset,
}

#[wasm_bindgen]
impl Pulse {
    /// Load snapshot bytes, gzip-compressed or raw.
    pub fn load(bytes: &[u8]) -> Result<Pulse, JsError> {
        console_error_panic_hook::set_once();
        let decoded;
        let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut buffer = Vec::new();
            GzDecoder::new(bytes)
                .read_to_end(&mut buffer)
                .map_err(|e| JsError::new(&format!("could not decompress the activity snapshot: {e}")))?;
            decoded = buffer;
            decoded.as_slice()
        } else {
            bytes
        };
        let snapshot = Snapshot::decode(bytes).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Pulse { dataset: Dataset::new(snapshot) })
    }

    /// Answer a dashboard API URL (such as `/api/dashboard?days=30`) with a JSON string.
    /// `today` is the viewer's UTC date (`YYYY-MM-DD`). Errors are thrown as `Error`
    /// objects with the detail as message and an HTTP-style `status` property.
    pub fn request(&self, url: &str, today: &str) -> Result<String, JsValue> {
        let today = parse_date(today).unwrap_or_else(|| day_of((js_sys::Date::now() / 1000.0) as i64));
        match self.dataset.request(url, today) {
            Ok(value) => Ok(value.to_string()),
            Err(error) => {
                let js_error = js_sys::Error::new(&error.detail);
                js_sys::Reflect::set(&js_error, &"status".into(), &JsValue::from(error.status))?;
                Err(js_error.into())
            }
        }
    }

    /// Snapshot metadata as JSON: version, generated_at, raw_generated_at, revisions.
    pub fn meta(&self) -> String {
        self.dataset.meta().to_string()
    }
}
