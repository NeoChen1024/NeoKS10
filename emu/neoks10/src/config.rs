//! The configuration file (docs/emulator.md, "Configuration file").
//!
//! Paths in the file are relative to the directory of the file. Unknown
//! keys are errors, so that a typing error does not pass silently.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub machine: MachineCfg,
    /// Disk units by number: `[disk.0]`.
    #[serde(default)]
    pub disk: BTreeMap<String, ImageCfg>,
    /// Tape transports by number: `[tape.0]`.
    #[serde(default)]
    pub tape: BTreeMap<String, ImageCfg>,
    pub boot: Option<BootCfg>,
    #[serde(default)]
    pub control: ControlCfg,
    pub dz11: Option<DzCfg>,
    /// The directory of the file, for the relative paths.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// `[dz11]`: the listeners of the 8 lines.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DzCfg {
    /// `HOST:PORT` of line 0; line k listens on PORT + k.
    pub listen: String,
}

impl DzCfg {
    /// The address of each line.
    pub fn addresses(&self) -> Result<Vec<String>, String> {
        let bad = || format!("[dz11] listen = \"{}\": HOST:PORT expected", self.listen);
        let (host, port) = self.listen.rsplit_once(':').ok_or_else(bad)?;
        let port: u16 = port.parse().map_err(|_| bad())?;
        if port > u16::MAX - 7 {
            return Err(format!("[dz11] listen: port {port} + 7 is above 65535"));
        }
        Ok((0..8).map(|k| format!("{host}:{}", port + k)).collect())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineCfg {
    /// The memory size in Ki words. The emulator has 512 Ki words.
    #[serde(default = "default_memory")]
    pub memory_kwords: usize,
    /// The virtual time of one instruction, in ns (docs/emulator.md,
    /// "Time").
    #[serde(default = "default_ns")]
    pub time_per_instruction_ns: u64,
}

fn default_memory() -> usize {
    ks10_core_soft::machine::MEMORY_WORDS / 1024
}

fn default_ns() -> u64 {
    1000
}

impl Default for MachineCfg {
    fn default() -> Self {
        MachineCfg {
            memory_kwords: default_memory(),
            time_per_instruction_ns: default_ns(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageCfg {
    pub image: PathBuf,
    #[serde(default)]
    pub read_only: bool,
    /// The disk type; only "rp06" exists.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BootDevice {
    Disk,
    Tape,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootCfg {
    pub device: BootDevice,
    #[serde(default)]
    pub unit: u16,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCfg {
    /// The path of the control socket.
    pub socket: Option<PathBuf>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut c = Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        c.dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(c)
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        c.check()?;
        Ok(c)
    }

    fn check(&self) -> Result<(), String> {
        if self.machine.memory_kwords != default_memory() {
            return Err(format!(
                "machine.memory_kwords = {}: the emulator has {} Ki words only",
                self.machine.memory_kwords,
                default_memory()
            ));
        }
        if self.machine.time_per_instruction_ns == 0 {
            return Err("machine.time_per_instruction_ns must not be 0".into());
        }
        for (kind, map) in [("disk", &self.disk), ("tape", &self.tape)] {
            for (k, img) in map {
                unit_number(k).ok_or_else(|| format!("{kind}.{k}: the unit must be 0 to 7"))?;
                if let Some(t) = &img.kind
                    && (kind == "tape" || t != "rp06")
                {
                    return Err(format!("{kind}.{k}: type \"{t}\" is not known"));
                }
            }
        }
        if let Some(b) = self.boot
            && b.unit > 7
        {
            return Err("boot.unit must be 0 to 7".into());
        }
        Ok(())
    }

    /// A path of the file, relative to its directory.
    pub fn path(&self, p: &Path) -> PathBuf {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dir.join(p)
        }
    }

    /// The units of a map with their numbers.
    pub fn units(map: &BTreeMap<String, ImageCfg>) -> impl Iterator<Item = (usize, &ImageCfg)> {
        map.iter()
            .filter_map(|(k, v)| unit_number(k).map(|n| (n, v)))
    }
}

fn unit_number(k: &str) -> Option<usize> {
    k.parse::<usize>().ok().filter(|&n| n < 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The example of docs/emulator.md, "Configuration file" (without the
    // sections that do not exist yet).
    #[test]
    fn example() {
        let c = Config::parse(
            r#"
            [machine]
            memory_kwords = 512
            time_per_instruction_ns = 1000

            [disk.0]
            type = "rp06"
            image = "rp0.dsk"

            [tape.0]
            image = "minsys.tape"
            read_only = true

            [boot]
            device = "disk"
            "#,
        )
        .unwrap();
        assert_eq!(c.disk["0"].image, PathBuf::from("rp0.dsk"));
        assert!(c.tape["0"].read_only);
        assert_eq!(c.boot.unwrap().device, BootDevice::Disk);
    }

    #[test]
    fn errors() {
        assert!(Config::parse("[machine]\nmemory_kwords = 256\n").is_err());
        assert!(Config::parse("[tape.8]\nimage = \"x\"\n").is_err());
        assert!(Config::parse("[disk.0]\nimage = \"x\"\ntype = \"rp07\"\n").is_err());
        // A typing error in a key.
        assert!(Config::parse("[disk.0]\nimgae = \"x\"\n").is_err());
    }

    #[test]
    fn dz_addresses() {
        let c = Config::parse("[dz11]\nlisten = \"127.0.0.1:10004\"\n").unwrap();
        let a = c.dz11.unwrap().addresses().unwrap();
        assert_eq!(a[0], "127.0.0.1:10004");
        assert_eq!(a[7], "127.0.0.1:10011");
        let c = Config::parse("[dz11]\nlisten = \"[::1]:65530\"\n").unwrap();
        assert!(c.dz11.unwrap().addresses().is_err());
        let c = Config::parse("[dz11]\nlisten = \"localhost\"\n").unwrap();
        assert!(c.dz11.unwrap().addresses().is_err());
    }
}
