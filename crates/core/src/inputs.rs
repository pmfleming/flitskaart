//! Where recorded sound comes from: the system's microphones and which one is
//! the default, read from PipeWire with `pw-dump`.

use std::process::Command;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Bluetooth,
    Usb,
    /// On the laptop's own sound chip: the built-in microphone or the headphone jack.
    BuiltIn,
    Other,
}

impl InputKind {
    pub fn label(self) -> &'static str {
        match self {
            InputKind::Bluetooth => "Bluetooth",
            InputKind::Usb => "USB",
            InputKind::BuiltIn => "laptop",
            InputKind::Other => "overig",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// PipeWire's node name, such as `bluez_input.80:99:E7:D7:98:EE`.
    pub id: String,
    /// The name people see, such as `WF-1000XM5`.
    pub label: String,
    pub kind: InputKind,
    /// 0.0–1.0 as the system's volume slider shows it.
    pub volume: Option<f32>,
    pub muted: bool,
    pub is_default: bool,
}

/// The microphones, with the default one first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs(pub Vec<Input>);

impl Inputs {
    /// Asks PipeWire. `None` when `pw-dump` is missing or fails.
    pub fn current() -> Option<Inputs> {
        let output = Command::new("pw-dump").output().ok()?;
        if !output.status.success() {
            return None;
        }
        parse(&String::from_utf8_lossy(&output.stdout))
    }

    /// The microphone recordings come from.
    pub fn default_input(&self) -> Option<&Input> {
        self.0.first().filter(|input| input.is_default)
    }

    pub fn others(&self) -> impl Iterator<Item = &Input> {
        self.0.iter().filter(|input| !input.is_default)
    }
}

/// Reads `pw-dump` output.
pub fn parse(json: &str) -> Option<Inputs> {
    let objects: Vec<Value> = serde_json::from_str(json).ok()?;
    let default_name = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Metadata")
        .filter_map(|o| o["metadata"].as_array())
        .flatten()
        .find(|entry| entry["key"] == "default.audio.source")
        .and_then(|entry| entry["value"]["name"].as_str())
        .map(str::to_string);
    // A node names its device by id, as a number or a string.
    let device_bus = |device_id: &Value| {
        let wanted = device_id.as_u64().or_else(|| device_id.as_str()?.parse().ok())?;
        objects
            .iter()
            .find(|o| o["type"] == "PipeWire:Interface:Device" && o["id"].as_u64() == Some(wanted))
            .and_then(|o| o["info"]["props"]["device.bus"].as_str())
            .map(str::to_string)
    };

    let mut inputs: Vec<Input> = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Node" && o["info"]["props"]["media.class"] == "Audio/Source")
        .filter_map(|o| {
            let props = &o["info"]["props"];
            let id = props["node.name"].as_str()?.to_string();
            let label = props["node.description"].as_str().or(props["node.nick"].as_str()).unwrap_or(&id).to_string();
            let bus = props["device.bus"].as_str().map(str::to_string).or_else(|| device_bus(&props["device.id"]));
            let kind = if id.starts_with("bluez") || bus.as_deref() == Some("bluetooth") {
                InputKind::Bluetooth
            } else if bus.as_deref() == Some("usb") {
                InputKind::Usb
            } else if bus.as_deref() == Some("pci") {
                InputKind::BuiltIn
            } else {
                InputKind::Other
            };
            let sound = o["info"]["params"]["Props"].as_array().and_then(|props| props.first());
            let muted = sound.and_then(|p| p["mute"].as_bool()).unwrap_or(false);
            // PipeWire keeps volumes cubed; the slider shows the cube root.
            let volume = sound
                .and_then(|p| p["channelVolumes"].as_array())
                .and_then(|channels| channels.iter().filter_map(Value::as_f64).reduce(f64::max))
                .map(|v| v.cbrt() as f32);
            let is_default = default_name.as_deref() == Some(id.as_str());
            Some(Input { id, label, kind, volume, muted, is_default })
        })
        .collect();
    inputs.sort_by_key(|input| !input.is_default);
    Some(Inputs(inputs))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"[
      { "id": 49, "type": "PipeWire:Interface:Device",
        "info": { "props": { "device.description": "Ryzen HD Audio Controller", "device.bus": "pci" } } },
      { "id": 75, "type": "PipeWire:Interface:Device",
        "info": { "props": { "device.description": "WF-1000XM5", "device.bus": "bluetooth" } } },
      { "id": 70, "type": "PipeWire:Interface:Node",
        "info": { "props": { "media.class": "Audio/Source", "node.name": "alsa_input.pci.Mic1",
                             "node.description": "Ryzen HD Audio Controller Digital Microphone", "device.id": 49 },
                  "params": { "Props": [ { "mute": true, "channelVolumes": [1.0, 1.0] }, {} ] } } },
      { "id": 65, "type": "PipeWire:Interface:Node",
        "info": { "props": { "media.class": "Audio/Source", "node.name": "bluez_input.80:99",
                             "node.description": "WF-1000XM5", "device.id": "75" },
                  "params": { "Props": [ { "mute": false, "channelVolumes": [0.125] } ] } } },
      { "id": 68, "type": "PipeWire:Interface:Node",
        "info": { "props": { "media.class": "Audio/Sink", "node.name": "alsa_output.speaker" } } },
      { "id": 31, "type": "PipeWire:Interface:Metadata",
        "metadata": [ { "subject": 0, "key": "default.audio.source", "value": { "name": "bluez_input.80:99" } } ] }
    ]"#;

    #[test]
    fn the_default_microphone_comes_first_with_its_kind_volume_and_mute() {
        let inputs = parse(DUMP).unwrap();
        let default = inputs.default_input().unwrap();
        assert_eq!(default.label, "WF-1000XM5");
        assert_eq!(default.kind, InputKind::Bluetooth);
        assert!(!default.muted);
        assert!((default.volume.unwrap() - 0.5).abs() < 1e-6);

        let others: Vec<_> = inputs.others().collect();
        assert_eq!(others.len(), 1, "speakers are not microphones");
        assert_eq!(others[0].label, "Ryzen HD Audio Controller Digital Microphone");
        assert_eq!(others[0].kind, InputKind::BuiltIn);
        assert!(others[0].muted);
    }

    #[test]
    fn without_a_default_no_microphone_is_claimed_as_the_source() {
        let inputs = parse(&DUMP.replace("default.audio.source", "default.video.source")).unwrap();
        assert!(inputs.default_input().is_none());
        assert_eq!(inputs.0.len(), 2);
    }
}
