//! Bounded Images SSE reader. Previews are ephemeral; only a complete terminal
//! stream can be passed to the ordinary local-save transaction.
use crate::{
    dto::ImageParameters,
    error::{Result, fail},
    images::MAX_RESPONSE_BYTES,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    b64_json: String,
    created_at: u64,
    partial_image_index: Option<u8>,
    background: Option<String>,
    quality: Option<String>,
    size: Option<String>,
    output_format: Option<String>,
    model: Option<String>,
}
pub struct Preview {
    pub payload: Vec<u8>,
    pub index: u8,
}
pub struct ImageStream {
    buffer: Vec<u8>,
    scanned: usize,
    data: Vec<u8>,
    name: String,
    total: u64,
    expected: usize,
    partials: u8,
    next: u8,
    prefix: &'static str,
    completed: Vec<Value>,
    metadata: Option<Value>,
    done: bool,
}
impl ImageStream {
    pub fn new(parameters: &ImageParameters, edit: bool) -> Self {
        Self {
            buffer: vec![],
            scanned: 0,
            data: vec![],
            name: String::new(),
            total: 0,
            expected: parameters.n.unwrap_or(1) as usize,
            partials: parameters.partial_images.unwrap_or(0),
            next: 0,
            prefix: if edit {
                "image_edit"
            } else {
                "image_generation"
            },
            completed: vec![],
            metadata: None,
            done: false,
        }
    }
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Preview>> {
        self.total = self
            .total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| fail("response_too_large"))?;
        if self.total > MAX_RESPONSE_BYTES {
            return Err(fail("response_too_large"));
        }
        self.buffer.extend_from_slice(chunk);
        let mut out = vec![];
        let mut start = 0;
        for end in self.scanned..self.buffer.len() {
            if self.buffer[end] != b'\n' {
                continue;
            }
            let line = self.buffer[start..end]
                .strip_suffix(b"\r")
                .unwrap_or(&self.buffer[start..end])
                .to_vec();
            if line.is_empty() {
                if let Some(preview) = self.dispatch()? {
                    out.push(preview);
                }
            } else if !line.starts_with(b":") {
                if let Some(split) = line.iter().position(|c| *c == b':') {
                    let key = &line[..split];
                    let value = line[split + 1..]
                        .strip_prefix(b" ")
                        .unwrap_or(&line[split + 1..]);
                    if key == b"event" {
                        if !self.name.is_empty() {
                            return Err(fail("invalid_image_response"));
                        }
                        self.name = String::from_utf8(value.to_vec())
                            .map_err(|_| fail("invalid_image_response"))?;
                    } else if key == b"data" {
                        if !self.data.is_empty() {
                            self.data.push(b'\n');
                        }
                        self.data.extend_from_slice(value);
                    }
                }
            }
            start = end + 1;
        }
        if start > 0 {
            self.buffer.drain(..start);
        }
        self.scanned = self.buffer.len();
        Ok(out)
    }
    fn dispatch(&mut self) -> Result<Option<Preview>> {
        if self.data.is_empty() {
            self.name.clear();
            return Ok(None);
        }
        let data = std::mem::take(&mut self.data);
        let name = std::mem::take(&mut self.name);
        if data == b"[DONE]" {
            if self.completed.len() != self.expected || self.done {
                return Err(fail("image_response_incomplete"));
            }
            self.done = true;
            return Ok(None);
        }
        if self.done || self.completed.len() >= self.expected {
            return Err(fail("invalid_image_response"));
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&data) {
            if name == "error" || value.get("error").is_some() {
                return Err(crate::images::generation_failure(502, &data));
            }
        }
        let event: Event =
            serde_json::from_slice(&data).map_err(|_| fail("invalid_image_response"))?;
        if !name.is_empty() && name != event.kind {
            return Err(fail("invalid_image_response"));
        }
        let partial = event.kind == format!("{}.partial_image", self.prefix);
        if !partial && event.kind != format!("{}.completed", self.prefix) {
            return Err(fail("invalid_image_response"));
        }
        let index = if partial {
            let index = event
                .partial_image_index
                .ok_or_else(|| fail("invalid_image_response"))?;
            if index != self.next || index >= self.partials {
                return Err(fail("invalid_image_response"));
            }
            self.next += 1;
            index
        } else {
            if event.partial_image_index.is_some() {
                return Err(fail("invalid_image_response"));
            }
            0
        };
        let mut payload = json!({"created":event.created_at,"data":[{"b64_json":event.b64_json}],"background":event.background,"quality":event.quality,"size":event.size,"output_format":event.output_format,"model":event.model});
        // A null optional field is omitted to retain the existing result parser.
        payload.as_object_mut().unwrap().retain(|_, v| !v.is_null());
        if partial {
            return Ok(Some(Preview {
                payload: serde_json::to_vec(&payload)
                    .map_err(|_| fail("invalid_image_response"))?,
                index,
            }));
        }
        self.completed.push(payload["data"][0].take());
        payload.as_object_mut().unwrap().remove("data");
        self.metadata = Some(payload);
        self.next = 0;
        Ok(None)
    }
    pub fn finish(mut self) -> Result<Vec<u8>> {
        if !self.buffer.is_empty() || !self.data.is_empty() {
            return Err(fail("image_response_incomplete"));
        }
        if self.completed.len() != self.expected || self.expected == 0 || self.expected > 10 {
            return Err(fail("image_response_incomplete"));
        }
        let mut payload = self
            .metadata
            .take()
            .ok_or_else(|| fail("image_response_incomplete"))?;
        payload["data"] = Value::Array(self.completed);
        serde_json::to_vec(&payload).map_err(|_| fail("invalid_image_response"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(kind: &str, partial: bool) -> Vec<u8> {
        let mut value = json!({"type":format!("image_generation.{kind}"),"created_at":1,"b64_json":"eA==","private_metadata":"private"});
        if partial {
            value["partial_image_index"] = json!(0);
        }
        format!("event: image_generation.{kind}\ndata: {value}\n\n").into_bytes()
    }
    #[test]
    fn chunk_boundaries_deliver_previews_without_claiming_a_completed_result() {
        let mut reader = ImageStream::new(
            &ImageParameters {
                stream: Some(true),
                partial_images: Some(1),
                ..Default::default()
            },
            false,
        );
        let mut previews = vec![];
        for byte in frame("partial_image", true) {
            previews.extend(reader.push(&[byte]).unwrap());
        }
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].index, 0);
        assert!(!String::from_utf8_lossy(&previews[0].payload).contains("private"));
        assert!(reader.finish().is_err());
        let mut complete = ImageStream::new(
            &ImageParameters {
                stream: Some(true),
                ..Default::default()
            },
            false,
        );
        for chunk in frame("completed", false).chunks(17) {
            assert!(complete.push(chunk).unwrap().is_empty());
        }
        let result = complete.finish().unwrap();
        let value: Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(value["data"].as_array().unwrap().len(), 1);
        assert!(!String::from_utf8_lossy(&result).contains("private"));
    }
    #[test]
    fn mismatched_or_duplicate_terminals_cannot_become_saved_images() {
        let mut duplicate = ImageStream::new(&ImageParameters::default(), false);
        duplicate.push(&frame("completed", false)).unwrap();
        assert!(duplicate.push(&frame("completed", false)).is_err());
        let mut wrong = ImageStream::new(&ImageParameters::default(), true);
        assert!(wrong.push(&frame("completed", false)).is_err());
        let mut missing = ImageStream::new(
            &ImageParameters {
                n: Some(2),
                ..Default::default()
            },
            false,
        );
        missing.push(&frame("completed", false)).unwrap();
        assert!(missing.finish().is_err());
    }
}
