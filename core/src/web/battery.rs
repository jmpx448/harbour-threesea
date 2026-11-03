// SPDX-License-Identifier: AGPL-3.0-only
//! The phone's battery for Threema Web (`update/batteryStatus`), from the kernel's power supply
//! class: the first supply of type `Battery`, its `capacity` and `status`.

use std::path::Path;

/// Charge in percent (if known) and whether it is charging; `None` without a battery.
#[must_use]
pub fn status(root: &Path) -> Option<(Option<u8>, bool)> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let read = |name: &str| {
            std::fs::read_to_string(path.join(name))
                .ok()
                .map(|s| s.trim().to_owned())
        };
        if read("type").as_deref() != Some("Battery") {
            continue;
        }
        let percent = read("capacity")
            .and_then(|c| c.parse::<u8>().ok())
            .filter(|p| *p <= 100);
        let charging = matches!(read("status").as_deref(), Some("Charging" | "Full"));
        return Some((percent, charging));
    }
    None
}

/// The system's power supplies.
pub const POWER_SUPPLY: &str = "/sys/class/power_supply";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_battery() {
        let dir = tempfile::tempdir().unwrap();
        let ac = dir.path().join("AC");
        std::fs::create_dir(&ac).unwrap();
        std::fs::write(ac.join("type"), "Mains\n").unwrap();
        assert_eq!(status(dir.path()), None);
        let battery = dir.path().join("battery");
        std::fs::create_dir(&battery).unwrap();
        std::fs::write(battery.join("type"), "Battery\n").unwrap();
        std::fs::write(battery.join("capacity"), "57\n").unwrap();
        std::fs::write(battery.join("status"), "Discharging\n").unwrap();
        assert_eq!(status(dir.path()), Some((Some(57), false)));
        std::fs::write(battery.join("status"), "Charging\n").unwrap();
        assert_eq!(status(dir.path()), Some((Some(57), true)));
    }
}
