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
