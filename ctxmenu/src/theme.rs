//! Appearance: the three-way theme choice and the Windows title bar.
//!
//! eframe lets Windows draw the window frame, so the title bar stays light
//! until DWM is told otherwise — a dark app with a white title bar is the
//! giveaway that a tool was not built for Windows (ToDo 9.4).

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
};
use windows::Win32::System::SystemServices::LANG_GERMAN;
use windows::Win32::UI::WindowsAndMessaging::{
    SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
};
use windows::core::BOOL;

use crate::settings::{Language, ThemeChoice};

/// Attribute 20 is the documented one, but it only exists from Windows 10
/// build 18985. Builds 1809 and 1903 use 19 instead, and `windows` 0.62 has no
/// named constant for the old spelling — its `DWMWA_*` block skips from 17 to
/// 20 — so the newtype is built by hand. Before build 17763 neither works and
/// the title bar simply stays light, which is acceptable (ToDo 9.4).
const DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(19);

impl ThemeChoice {
    pub fn to_preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// Reads the Win32 window handle out of anything eframe hands us.
///
/// Both `CreationContext` and `Frame` implement `HasWindowHandle` on native
/// targets, so this works at startup and on every later theme switch.
pub fn window_handle(source: &impl HasWindowHandle) -> Option<HWND> {
    match source.window_handle().ok()?.as_raw() {
        // `Win32WindowHandle::hwnd` is a `NonZeroIsize`, while `HWND` in
        // windows 0.62 is a newtype over `*mut c_void`.
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut c_void)),
        _ => None,
    }
}

/// Switches the DWM title bar to dark or light.
///
/// Returns whether Windows accepted the change, so the caller can record what
/// actually happened rather than assuming.
pub fn set_titlebar_dark(hwnd: HWND, dark: bool) -> bool {
    // DWM wants a four-byte Win32 BOOL, not a one-byte Rust bool.
    let value: BOOL = dark.into();
    let pointer = std::ptr::addr_of!(value) as *const c_void;
    let size = std::mem::size_of::<BOOL>() as u32;

    unsafe {
        let applied = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, pointer, size)
            .is_ok()
            || DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1,
                pointer,
                size,
            )
            .is_ok();

        if applied {
            // On some builds the title bar only flips after the frame is
            // redrawn, hence the no-op move that carries SWP_FRAMECHANGED.
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }

        applied
    }
}

/// Where Windows keeps the light/dark choice for applications.
///
/// `winit` does not read this key: it calls ordinal 132 of `uxtheme.dll`
/// (`ShouldAppsUseDarkMode`), which reports the same setting. Writing here is
/// therefore the way to move what `winit` sees, and the broadcast below is
/// what tells it to look again.
const PERSONALIZE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const APPS_USE_LIGHT: &str = "AppsUseLightTheme";

/// Reads the current setting. `1` is light, `0` is dark, absent means light.
pub fn apps_use_light_theme() -> Option<u32> {
    windows_registry::CURRENT_USER
        .open(PERSONALIZE)
        .ok()?
        .get_u32(APPS_USE_LIGHT)
        .ok()
}

/// Sets the light/dark choice and tells every window about it.
///
/// The broadcast is not decoration. Windows sends `WM_SETTINGCHANGE` itself
/// when the setting is changed through the Settings app; a bare registry write
/// changes the value and nothing else, so a window would keep its old
/// appearance until something happened to make it ask again. Sending the same
/// message with `ImmersiveColorSet` is exactly what the shell does.
fn set_apps_use_light_theme(light: u32) -> anyhow::Result<()> {
    windows_registry::CURRENT_USER
        .create(PERSONALIZE)?
        .set_u32(APPS_USE_LIGHT, light)?;
    broadcast_colour_change();
    Ok(())
}

fn broadcast_colour_change() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    let name = windows::core::w!("ImmersiveColorSet");
    unsafe {
        // A timeout, and ABORTIFHUNG: this goes to every top-level window on
        // the desktop, and one that is not pumping messages must not take this
        // process down with it.
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(name.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            200,
            None,
        );
    }
}

/// Flips the system theme and puts it back when dropped.
///
/// The whole desktop follows this setting, so leaving it flipped because a
/// probe panicked would be rude in a way the user notices immediately. `Drop`
/// runs while unwinding, which is the case this exists for.
pub struct SystemThemeGuard {
    original: Option<u32>,
}

impl SystemThemeGuard {
    /// Switches to the opposite of what is set now.
    ///
    /// Returns the guard and what the setting was moved to, or an error if the
    /// value could not be written — in which case nothing was changed.
    pub fn flip() -> anyhow::Result<(Self, bool)> {
        let original = apps_use_light_theme();
        // Absent counts as light, which is what Windows does.
        let was_light = original.unwrap_or(1) == 1;
        let now_light = u32::from(!was_light);

        set_apps_use_light_theme(now_light)?;
        Ok((Self { original }, now_light == 1))
    }
}

impl Drop for SystemThemeGuard {
    fn drop(&mut self) {
        let restored = match self.original {
            Some(value) => set_apps_use_light_theme(value),
            // The value did not exist before; removing it again is the honest
            // restore, and the broadcast still has to go out.
            None => windows_registry::CURRENT_USER
                .create(PERSONALIZE)
                .and_then(|key| key.remove_value(APPS_USE_LIGHT))
                .map_err(anyhow::Error::from)
                .inspect(|_| broadcast_colour_change()),
        };
        if let Err(error) = restored {
            crate::errln!("theme_probe: RESTORE FAILED, system left flipped: {error:#}");
            crate::console::flush();
        }
    }
}

/// The start language, taken from Windows.
///
/// `windows` 0.62 exposes no `PRIMARYLANGID` helper, so the primary language
/// is masked out by hand. Everything but German falls back to English.
pub fn system_language() -> Language {
    let lang_id = unsafe { GetUserDefaultUILanguage() };
    Language::from_ui_language(lang_id)
}

/// Is the primary language German, by the same rule the settings use?
///
/// Exists so the mask constant is asserted against the Win32 one in a test
/// rather than trusted twice.
pub fn german_primary_id() -> u16 {
    LANG_GERMAN as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hand_written_mask_matches_the_win32_constant() {
        // settings::Language::from_ui_language hard-codes 0x07 so it can be
        // tested without Windows types; this pins that number to the real one.
        assert_eq!(german_primary_id(), 0x07);
    }

    #[test]
    fn the_system_language_is_one_of_the_two_supported_ones() {
        // Cannot assert which one without knowing the machine, but it must
        // resolve rather than panic, and must agree with the pure function.
        let from_win32 = system_language();
        let raw = unsafe { GetUserDefaultUILanguage() };
        assert_eq!(from_win32, Language::from_ui_language(raw));
    }

    #[test]
    fn the_probe_reads_the_setting_windows_actually_uses() {
        // Not an assertion about which theme is set — that is the user's
        // business — but that the value is readable and one of the two
        // meanings Windows gives it. A probe that silently read nothing would
        // flip to "light" every time and prove the wrong thing.
        if let Some(value) = apps_use_light_theme() {
            assert!(value <= 1, "unexpected AppsUseLightTheme: {value}");
        }
    }

    /// Not part of the ordinary run, and deliberately so: this test switches
    /// the real desktop twice and takes about 18 seconds doing it, because the
    /// `WM_SETTINGCHANGE` broadcast waits on every top-level window there is.
    /// A test suite that repaints the developer's screen every time would soon
    /// stop being run at all.
    ///
    /// `cargo test -- --ignored the_guard_puts_the_setting_back`
    #[test]
    #[ignore = "flips the real system theme; run it deliberately"]
    fn the_guard_puts_the_setting_back() {
        // The one property that matters: the desktop must look the same
        // afterwards. Runs against the real setting, because a mock would
        // prove only that the mock works.
        let before = apps_use_light_theme();

        {
            let (_guard, now_light) = match SystemThemeGuard::flip() {
                Ok(pair) => pair,
                // A machine where this key cannot be written is not a failure
                // of the guard.
                Err(_) => return,
            };
            assert_eq!(
                apps_use_light_theme(),
                Some(u32::from(now_light)),
                "the flip must actually reach the registry"
            );
            assert_ne!(
                apps_use_light_theme(),
                before,
                "flipping to the same value would measure nothing"
            );
        }

        assert_eq!(
            apps_use_light_theme(),
            before,
            "the guard must leave the setting exactly as it found it"
        );
    }

    #[test]
    fn every_theme_choice_maps_to_a_preference() {
        assert_eq!(
            ThemeChoice::System.to_preference(),
            egui::ThemePreference::System
        );
        assert_eq!(
            ThemeChoice::Light.to_preference(),
            egui::ThemePreference::Light
        );
        assert_eq!(
            ThemeChoice::Dark.to_preference(),
            egui::ThemePreference::Dark
        );
    }
}
