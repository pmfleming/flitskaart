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
    const LABELS: [&str; 4] = ["Bluetooth", "USB", "laptop", "overig"];

    /// From the node's name and its device's bus.
    fn of(node_name: &str, bus: Option<&str>) -> InputKind {
        if node_name.starts_with("bluez") {
            return InputKind::Bluetooth;
        }
        match bus {
            Some("bluetooth") => InputKind::Bluetooth,
            Some("usb") => InputKind::Usb,
            Some("pci") => InputKind::BuiltIn,
            _ => InputKind::Other,
        }
    }

    pub fn label(self) -> &'static str {
        InputKind::LABELS[self as usize]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// Stable PipeWire node name, used to target capture without changing the system default.
    pub node: String,
    /// The name people see, such as `WF-1000XM5`.
    pub label: String,
    pub kind: InputKind,
    /// 0.0–1.0 as the system's volume slider shows it.
    pub volume: Option<f32>,
    pub muted: bool,
    pub is_default: bool,
}

impl Input {
    /// "Bluetooth, volume 100%, gedempt".
    pub fn describe(&self) -> String {
        let volume = self.volume.map(|volume| format!("volume {:.0}%", volume * 100.0));
        let muted = self.muted.then(|| "gedempt".to_string());
        [Some(self.kind.label().to_string()), volume, muted].into_iter().flatten().collect::<Vec<_>>().join(", ")
    }
}

/// The microphones, with the default one first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs(pub Vec<Input>);

impl Inputs {
    /// Asks PipeWire. `None` when `pw-dump` is missing or fails.
    pub fn current() -> Option<Inputs> {
        let output = Command::new("pw-dump").output().ok().filter(|output| output.status.success())?;
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

/// The objects in a `pw-dump`.
struct Dump(Vec<Value>);

impl Dump {
    fn of_type<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Value> {
        self.0.iter().filter(move |object| object["type"] == kind)
    }

    /// The node name of the default microphone.
    fn default_source(&self) -> Option<&str> {
        self.of_type("PipeWire:Interface:Metadata")
            .filter_map(|object| object["metadata"].as_array())
            .flatten()
            .find(|entry| entry["key"] == "default.audio.source")?["value"]["name"]
            .as_str()
    }

    /// The bus of the device a node names by id, as a number or a string.
    fn device_bus(&self, device_id: &Value) -> Option<&str> {
        let wanted = device_id.as_u64().or_else(|| device_id.as_str()?.parse().ok())?;
        self.of_type("PipeWire:Interface:Device").find(|object| object["id"].as_u64() == Some(wanted))?["info"]["props"]
            ["device.bus"]
            .as_str()
    }

    fn input(&self, node: &Value, default_source: Option<&str>) -> Option<Input> {
        let props = &node["info"]["props"];
        let name = props["node.name"].as_str()?;
        let label = props["node.description"].as_str().or(props["node.nick"].as_str()).unwrap_or(name);
        let bus = props["device.bus"].as_str().or_else(|| self.device_bus(&props["device.id"]));
        let sound = &node["info"]["params"]["Props"][0];
        // PipeWire keeps volumes cubed; the slider shows the cube root.
        let volume = sound["channelVolumes"].as_array().and_then(|channels| channels.iter().filter_map(Value::as_f64).reduce(f64::max));
        Some(Input {
            node: name.to_string(),
            label: label.to_string(),
            kind: InputKind::of(name, bus),
            volume: volume.map(|v| v.cbrt() as f32),
            muted: sound["mute"].as_bool().unwrap_or(false),
            is_default: default_source == Some(name),
        })
    }
}

/// Reads `pw-dump` output.
pub fn parse(json: &str) -> Option<Inputs> {
    let dump = Dump(serde_json::from_str(json).ok()?);
    let default_source = dump.default_source();
    let mut inputs: Vec<Input> = dump
        .of_type("PipeWire:Interface:Node")
        .filter(|node| node["info"]["props"]["media.class"] == "Audio/Source")
        .filter_map(|node| dump.input(node, default_source))
        .collect();
    inputs.sort_by_key(|input| !input.is_default);
    Some(Inputs(inputs))
}

#[cfg(test)]
mod tests {
    use super::{InputKind, parse};

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
        assert_eq!(default.describe(), "Bluetooth, volume 50%");

        let others: Vec<_> = inputs.others().collect();
        assert_eq!(others.len(), 1, "speakers are not microphones");
        assert_eq!(others[0].label, "Ryzen HD Audio Controller Digital Microphone");
        assert_eq!(others[0].kind, InputKind::BuiltIn);
        assert!(others[0].muted);
        assert_eq!(others[0].describe(), "laptop, volume 100%, gedempt");
    }

    #[test]
    fn without_a_default_no_microphone_is_claimed_as_the_source() {
        let inputs = parse(&DUMP.replace("default.audio.source", "default.video.source")).unwrap();
        assert!(inputs.default_input().is_none());
        assert_eq!(inputs.0.len(), 2);
    }
}
