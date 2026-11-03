use tracing::info;
use wm_common::{DisplayState, HideMethod};
use wm_platform::NativeWindow;
#[cfg(target_os = "windows")]
use wm_platform::NativeWindowWindowsExt;

use crate::{
  commands::window::unmanage_window, traits::WindowGetters,
  user_config::UserConfig, wm_state::WmState,
};

pub fn handle_window_hidden(
  native_window: &NativeWindow,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let found_window = state.window_from_native(native_window);

  if let Some(window) = found_window {
    info!("Window hidden: {window}");

    // The WM only cloaks with this hide method, so a window that lost
    // `WS_VISIBLE` was hidden by its application (e.g. closed to the
    // tray). Unmanage it whatever its display state. If the app hides it
    // while the WM is cloaking it, or while its workspace is hidden, the
    // checks below would keep it managed as an invisible window that
    // focus commands still land on.
    #[cfg(target_os = "windows")]
    if config.value.general.hide_method == HideMethod::Cloak
      && !window.native().has_visible_style()
    {
      unmanage_window(window, state)?;
      return Ok(());
    }

    // Update the display state.
    if config.value.general.hide_method != HideMethod::PlaceInCorner
      && window.display_state() == DisplayState::Hiding
    {
      window.set_display_state(DisplayState::Hidden);
      return Ok(());
    }

    // Unmanage the window if it's not in a display state transition. Also,
    // since window events are not 100% guaranteed to be in correct order,
    // we need to ignore events where the window is not actually hidden.
    if (config.value.general.hide_method == HideMethod::PlaceInCorner
      || window.display_state() == DisplayState::Shown)
      && !window.native().is_visible().unwrap_or(false)
    {
      unmanage_window(window, state)?;
    }
  }

  Ok(())
}
