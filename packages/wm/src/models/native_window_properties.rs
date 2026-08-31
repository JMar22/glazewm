use std::time::Instant;

use wm_platform::{NativeWindow, Rect};
#[cfg(target_os = "windows")]
use wm_platform::{NativeWindowWindowsExt, RectDelta};

#[derive(Debug, Clone)]
pub struct NativeWindowProperties {
  pub title: String,
  #[cfg(target_os = "windows")]
  pub class_name: String,
  pub process_name: String,
  pub frame: Rect,
  pub is_minimized: bool,
  pub is_maximized: bool,
  pub is_resizable: bool,
  #[cfg(target_os = "windows")]
  pub shadow_borders: RectDelta,

  /// When the WM read these properties, which is when it decided whether
  /// to manage the window. Used to tell an application placing its own
  /// window as it starts up from the user moving it later.
  ///
  /// These properties are read once per window and carried across state
  /// changes, so this does not move for the life of the window.
  pub read_at: Instant,
}

impl TryFrom<&NativeWindow> for NativeWindowProperties {
  type Error = anyhow::Error;

  fn try_from(native_window: &NativeWindow) -> Result<Self, Self::Error> {
    Ok(Self {
      title: native_window.title()?,
      #[cfg(target_os = "windows")]
      class_name: native_window.class_name()?,
      process_name: native_window.process_name()?,
      frame: native_window.frame()?,
      is_minimized: native_window.is_minimized()?,
      is_maximized: native_window.is_maximized()?,
      is_resizable: native_window.is_resizable()?,
      #[cfg(target_os = "windows")]
      shadow_borders: native_window.shadow_borders()?,
      read_at: Instant::now(),
    })
  }
}
