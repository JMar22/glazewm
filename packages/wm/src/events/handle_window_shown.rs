use tracing::info;
use wm_common::{DisplayState, HideMethod};
use wm_platform::NativeWindow;
#[cfg(target_os = "windows")]
use wm_platform::NativeWindowWindowsExt;

use crate::{
  commands::window::manage_window, traits::WindowGetters,
  user_config::UserConfig, wm_state::WmState,
};

pub fn handle_window_shown(
  native_window: NativeWindow,
  state: &mut WmState,
  config: &mut UserConfig,
) -> anyhow::Result<()> {
  let found_window = state.window_from_native(&native_window);

  if let Some(window) = found_window {
    info!("Window shown: {window}");

    // Update display state if window is already managed.
    if config.value.general.hide_method != HideMethod::PlaceInCorner
      && window.display_state() == DisplayState::Showing
    {
      window.set_display_state(DisplayState::Shown);
    } else {
      state.pending_sync.queue_container_to_redraw(window);
    }
  } else if !state.ignored_windows.contains(&native_window) {
    #[cfg(target_os = "windows")]
    remove_leftover_cloak(&native_window, state);

    // If the window is not managed and not explicitly ignored, manage it.
    manage_window(native_window, None, state, config)?;
  }

  Ok(())
}

/// Takes off a cloak the WM left on a window it unmanaged while the
/// window's application had it hidden.
///
/// Applications can flicker a window while it starts: a maximized winit
/// window is shown, hidden and shown again. So the record stays until the
/// window is really visible, which is when it has an application view.
#[cfg(target_os = "windows")]
fn remove_leftover_cloak(
  native_window: &NativeWindow,
  state: &mut WmState,
) {
  let window_id = native_window.id();

  if !state.windows_left_cloaked.contains(&window_id)
    || !native_window.has_visible_style()
  {
    return;
  }

  match native_window.set_cloaked(false) {
    Ok(()) => {
      state.windows_left_cloaked.remove(&window_id);
    }
    Err(err) => {
      tracing::warn!("Failed to remove leftover cloak: {}", err);
    }
  }
}
