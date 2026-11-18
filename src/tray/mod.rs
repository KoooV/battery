//! StatusNotifierItem integration.
//!
//! One tray item per device. A device that appears gets its own item, a device
//! that goes away loses it; that bookkeeping is added together with polling in
//! stage 4. Each item carries the device name and charge in its menu and a
//! "Quit" entry that stops the application.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};
use tracing::{info, warn};

use crate::icon;
use crate::poll::{Reading, TraySink};
use crate::sources::Device;

/// Label of the menu entry that stops the application.
const QUIT_LABEL: &str = "Quit";

/// A tray icon showing the charge level of one device.
pub struct DeviceTray {
    device: Device,
    level: Option<u8>,
    quit: Arc<AtomicBool>,
}

impl DeviceTray {
    /// Creates an item for a device with its last known level.
    pub fn new(device: Device, level: Option<u8>, quit: Arc<AtomicBool>) -> Self {
        Self {
            device,
            level,
            quit,
        }
    }

    /// Device this item belongs to.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Level shown in the menu and the tooltip.
    pub fn level_text(&self) -> String {
        match self.level {
            Some(level) => format!("{level}%"),
            None => "no data".to_owned(),
        }
    }

    /// Replaces the level; the tray re-reads it on the next update.
    pub fn set_level(&mut self, level: Option<u8>) {
        self.level = level;
    }

    /// Happens when the user picks "Quit"; the poll loop owns the shutdown.
    fn request_quit(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
    }
}

/// The set of tray icons the application currently shows.
///
/// Implements [`TraySink`], so the poll loop never talks to ksni directly: it
/// hands over readings and this type adds, updates or removes icons.
pub struct TrayItems {
    quit: Arc<AtomicBool>,
    items: HashMap<String, Handle<DeviceTray>>,
}

impl TrayItems {
    /// Creates an empty set of icons that reports quits through `quit`.
    pub fn new(quit: Arc<AtomicBool>) -> Self {
        Self {
            quit,
            items: HashMap::new(),
        }
    }

    /// Shuts every icon down; called when the application exits.
    pub fn shutdown(&mut self) {
        for (_, handle) in self.items.drain() {
            handle.shutdown().wait();
        }
    }
}

impl TraySink for TrayItems {
    fn show(&mut self, reading: &Reading) {
        let level = reading.state.level;

        if let Some(handle) = self.items.get(&reading.device.id) {
            handle.update(|tray: &mut DeviceTray| tray.set_level(level));
            return;
        }

        let tray = DeviceTray::new(reading.device.clone(), level, Arc::clone(&self.quit));
        match tray.spawn() {
            Ok(handle) => {
                info!("tray icon added for {}", reading.device.name);
                self.items.insert(reading.device.id.clone(), handle);
            }
            Err(err) => warn!("cannot add a tray icon for {}: {err}", reading.device.name),
        }
    }

    fn remove(&mut self, device_id: &str) {
        if let Some(handle) = self.items.remove(device_id) {
            info!("tray icon removed for {device_id}");
            handle.shutdown().wait();
        }
    }

    fn visible(&self) -> Vec<String> {
        self.items.keys().cloned().collect()
    }
}

/// Converts a rendered pixmap into the icon type the tray protocol uses.
fn icon_from(pixmap: tiny_skia::Pixmap) -> Icon {
    Icon {
        width: pixmap.width() as i32,
        height: pixmap.height() as i32,
        data: icon::to_argb32(&pixmap),
    }
}

impl Tray for DeviceTray {
    // Ids have to be unique across items of one application and stable between
    // sessions, so the device id is part of it.
    fn id(&self) -> String {
        format!("battery-tray-{}", self.device.id)
    }

    fn category(&self) -> Category {
        Category::Hardware
    }

    fn status(&self) -> Status {
        match self.level {
            Some(level) if level <= icon::CRITICAL_LEVEL => Status::NeedsAttention,
            _ => Status::Active,
        }
    }

    fn title(&self) -> String {
        format!("{}: {}", self.device.name, self.level_text())
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        icon::render(self.level)
            .map(icon_from)
            .into_iter()
            .collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.device.name.clone(),
            description: self.level_text(),
            ..ToolTip::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            // The device line is informational only.
            StandardItem {
                label: format!("{} — {}", self.device.name, self.level_text()),
                enabled: false,
                ..StandardItem::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: QUIT_LABEL.to_owned(),
                activate: Box::new(|tray: &mut Self| tray.request_quit()),
                ..StandardItem::default()
            }
            .into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{DeviceKind, Protocol};

    fn device(id: &str, name: &str) -> Device {
        Device::new(id, name, DeviceKind::Mouse, Protocol::Upower)
    }

    fn tray(level: Option<u8>) -> (DeviceTray, Arc<AtomicBool>) {
        let quit = Arc::new(AtomicBool::new(false));
        (
            DeviceTray::new(
                device("/dev/mouse0", "Test Mouse"),
                level,
                Arc::clone(&quit),
            ),
            quit,
        )
    }

    fn labels(tray: &DeviceTray) -> Vec<String> {
        tray.menu()
            .iter()
            .filter_map(|item| match item {
                MenuItem::Standard(item) => Some(item.label.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn id_is_stable_and_device_specific() {
        let (first, _) = tray(Some(50));
        let (second, _) = tray(Some(20));

        assert_eq!(first.id(), "battery-tray-/dev/mouse0");
        assert_eq!(first.id(), second.id());
        assert_ne!(first.id(), "battery-tray-");
    }

    #[test]
    fn menu_shows_device_and_level() {
        let (tray, _) = tray(Some(42));

        assert_eq!(labels(&tray), ["Test Mouse — 42%", "Quit"]);
    }

    #[test]
    fn menu_says_no_data_when_the_level_is_unknown() {
        let (tray, _) = tray(None);

        assert_eq!(labels(&tray), ["Test Mouse — no data", "Quit"]);
    }

    #[test]
    fn menu_info_line_is_not_clickable() {
        let (tray, _) = tray(Some(42));

        let info = tray
            .menu()
            .into_iter()
            .find_map(|item| match item {
                MenuItem::Standard(item) if item.enabled => Some(item),
                _ => None,
            })
            .expect("quit entry is enabled");
        assert_eq!(info.label, QUIT_LABEL);

        // The disabled entry exists as well.
        let disabled = tray
            .menu()
            .into_iter()
            .filter_map(|item| match item {
                MenuItem::Standard(item) if !item.enabled => Some(item),
                _ => None,
            })
            .count();
        assert_eq!(disabled, 1);
    }

    #[test]
    fn quit_entry_asks_the_application_to_stop() {
        let (mut tray, quit) = tray(Some(42));

        let entry = tray
            .menu()
            .into_iter()
            .find_map(|item| match item {
                MenuItem::Standard(item) if item.label == QUIT_LABEL => Some(item),
                _ => None,
            })
            .expect("quit entry must exist");

        assert!(!quit.load(Ordering::Relaxed));
        (entry.activate)(&mut tray);
        assert!(quit.load(Ordering::Relaxed));
    }

    #[test]
    fn icon_is_provided_and_has_the_expected_size() {
        let (tray, _) = tray(Some(42));
        let icons = tray.icon_pixmap();

        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].width, icon::ICON_SIZE as i32);
        assert_eq!(
            icons[0].data.len(),
            (icon::ICON_SIZE * icon::ICON_SIZE * 4) as usize
        );
    }

    #[test]
    fn title_and_tooltip_carry_the_device_state() {
        let (tray, _) = tray(Some(42));

        assert_eq!(tray.title(), "Test Mouse: 42%");
        let tip = tray.tool_tip();
        assert_eq!(tip.title, "Test Mouse");
        assert_eq!(tip.description, "42%");
    }

    #[test]
    fn low_level_asks_for_attention() {
        assert_eq!(
            tray(Some(icon::CRITICAL_LEVEL)).0.status(),
            Status::NeedsAttention
        );
        assert_eq!(
            tray(Some(icon::CRITICAL_LEVEL + 1)).0.status(),
            Status::Active
        );
        assert_eq!(tray(None).0.status(), Status::Active);
    }

    #[test]
    fn level_can_be_updated() {
        let (mut tray, _) = tray(Some(42));

        tray.set_level(Some(9));

        assert_eq!(tray.level_text(), "9%");
        assert_eq!(tray.status(), Status::NeedsAttention);
    }
}
