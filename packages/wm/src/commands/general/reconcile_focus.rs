//! Recovery for keyboard focus stranded on the Windows shell.
//!
//! `handle_window_focused` restores focus when a foreground event tells us
//! the shell took it. Some transitions never produce that event: when a
//! taskbar flyout (Widgets, Search, the wifi/audio/battery quick settings)
//! is dismissed, Windows either drops the foreground entirely or leaves it
//! on the flyout's now-cloaked host window. `EVENT_SYSTEM_FOREGROUND` does
//! not fire for either, so focus sits nowhere until the user clicks a real
//! window. This polls for that state instead of waiting for an event that
//! never arrives.

#[cfg(target_os = "windows")]
use tracing::info;
#[cfg(target_os = "windows")]
use wm_common::DisplayState;
#[cfg(target_os = "windows")]
use wm_platform::{
  Dispatcher, DispatcherExtWindows, NativeWindow, NativeWindowWindowsExt,
};

#[cfg(target_os = "windows")]
use crate::traits::{CommonGetters, WindowGetters};
use crate::{user_config::UserConfig, wm_state::WmState};

/// Number of consecutive polls that must observe stranded focus before it
/// is restored. Debouncing avoids racing the brief foreground gaps that
/// occur during normal window activation.
#[cfg(target_os = "windows")]
const STRANDED_FOCUS_TICKS_BEFORE_RESTORE: u32 = 2;

/// Restores attempted for one stranded stretch before giving up.
///
/// Windows grants foreground rights to the process that last received
/// input. When the shell refuses `SetForegroundWindow` it keeps refusing
/// until the user interacts, so retrying forever cannot recover focus and
/// sends an input event every couple of seconds to no purpose.
#[cfg(target_os = "windows")]
const STRANDED_FOCUS_MAX_ATTEMPTS: u32 = 3;

/// Restores focus to the WM's focused window when the OS has left keyboard
/// focus stranded on the shell.
///
/// A failed restore is logged rather than propagated, but the fallible
/// signature is kept so this matches the other arms of the main event
/// loop.
#[cfg(target_os = "windows")]
#[allow(clippy::unnecessary_wraps)]
pub fn reconcile_stranded_focus(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  if state.is_paused || !config.value.general.restore_focus_on_shell {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  // A locked session has no user to hand focus back to, and the lock
  // screen legitimately holds the foreground, so every poll would read
  // as stranded. Restoring focus calls `SendInput` to defeat the
  // foreground lock, which Windows counts as user input and uses to
  // wake the display -- polling that against a lock screen turns the
  // display back on within seconds of it going off, indefinitely.
  //
  // A failed query is treated as unlocked so that a broken lock check
  // degrades to the previous behaviour rather than disabling recovery.
  if state.dispatcher.is_session_locked().unwrap_or(false) {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  // An open menu holds the foreground on the window that owns it, and that
  // owner is typically an invisible message-only window -- a tray icon's
  // window, or the hidden owner behind the Win+X menu. That reads as
  // stranded focus by every test below, so without this the poll restores
  // focus out from under a menu the user is working their way through,
  // roughly a second and a half after it opens. Restoring focus also sends
  // input to defeat the foreground lock, which dismisses the menu
  // outright.
  //
  // A failed query is treated as no menu so that a broken check degrades
  // to the previous behaviour rather than disabling recovery.
  if state.dispatcher.is_menu_open().unwrap_or(false) {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  // Only restore to a window the user can actually see. A hidden or
  // minimized target means there is nothing to return focus to.
  let target = state
    .focused_container()
    .and_then(|container| container.as_window_container().ok())
    .filter(|window| {
      matches!(
        window.display_state(),
        DisplayState::Shown | DisplayState::Showing
      )
    })
    .map(|window| window.native().clone())
    .filter(|native| {
      native.is_visible().unwrap_or(false)
        && !native.is_minimized().unwrap_or(true)
    });

  let (Some(target), Ok(current)) =
    (target, state.dispatcher.focused_window())
  else {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  };

  if current == target || !is_stranded_focus_target(&current) {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  // Checked only once focus already looks stranded, which is rare, so the
  // window enumeration below costs nothing on the common path.
  if shell_popup_is_open(&state.dispatcher) {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  state.stranded_focus_ticks += 1;
  if state.stranded_focus_ticks < STRANDED_FOCUS_TICKS_BEFORE_RESTORE {
    return Ok(());
  }

  state.stranded_focus_ticks = 0;

  // Stop once the shell has refused often enough to show it will keep
  // refusing. The counter is cleared as soon as focus lands anywhere
  // real, so a later stranding gets a fresh set of attempts.
  if state.stranded_focus_attempts >= STRANDED_FOCUS_MAX_ATTEMPTS {
    return Ok(());
  }

  state.stranded_focus_attempts += 1;
  info!("Restoring focus stranded on the Windows shell.");

  match target.focus() {
    Ok(()) => state.stranded_focus_attempts = 0,
    Err(err) => {
      tracing::warn!("Failed to restore stranded focus: {err}");

      if state.stranded_focus_attempts >= STRANDED_FOCUS_MAX_ATTEMPTS {
        tracing::warn!(
          "Giving up on stranded focus after {STRANDED_FOCUS_MAX_ATTEMPTS} attempts; the shell holds the foreground until there is user input."
        );
      }
    }
  }

  Ok(())
}

#[cfg(not(target_os = "windows"))]
#[allow(clippy::unnecessary_wraps)]
pub fn reconcile_stranded_focus(
  _state: &mut WmState,
  _config: &UserConfig,
) -> anyhow::Result<()> {
  Ok(())
}

/// Whether keyboard focus currently sits somewhere the user cannot type.
///
/// This is deliberately wider than [`is_shell_focus_target`]: a dismissed
/// taskbar flyout can remain the foreground window while cloaked.
/// `is_visible` already accounts for cloaking, so an *open* flyout is
/// visible and uncloaked and never matches here — it keeps its focus and
/// stays usable.
#[cfg(target_os = "windows")]
pub(crate) fn is_stranded_focus_target(
  native_window: &NativeWindow,
) -> bool {
  is_shell_focus_target(native_window)
    || !native_window.is_visible().unwrap_or(true)
}

/// Window class of the popup the shell draws its menus into.
///
/// Windows 11 builds the Win+X menu -- the one reached by right-clicking
/// Start, with "Shut down or sign out" and its submenu -- out of XAML
/// popups rather than the classic menus of earlier versions. Nothing about
/// it looks like a menu to the window manager: the foreground stays on
/// `Shell_TrayWnd`, and no thread in the shell enters menu mode, so
/// [`DispatcherExtWindows::is_menu_open`] cannot see it. The popup window
/// itself is the only evidence that the menu is on screen.
#[cfg(any(test, target_os = "windows"))]
pub(crate) const SHELL_POPUP_WINDOW_CLASS: &str =
  "Xaml_WindowedPopupClass";

/// How much taller than the taskbar a popup must be to be a menu.
///
/// The shell draws its tooltips into the same window class as its menus,
/// owned by the same taskbar, with identical window and extended styles --
/// there is nothing in the window itself that says which is which. Size is
/// the one thing that separates them: measured against a 40px taskbar, a
/// tooltip is 41px tall while the Win+X menu is 608px and its "Shut down
/// or sign out" submenu 154px. Comparing against the taskbar rather than
/// against a pixel count keeps that true whatever the display scaling.
///
/// This is a heuristic, and the consequence of it being wrong is bounded
/// in the safe direction: too low and a stranded tooltip delays focus
/// recovery, too high and a menu is dismissed as it was before.
#[cfg(any(test, target_os = "windows"))]
pub(crate) const MENU_POPUP_MIN_TASKBAR_HEIGHTS: i32 = 2;

/// Whether the shell is showing a popup menu.
///
/// The shell destroys these windows as its menus close, so this is only
/// true while one is on screen. Focus left on the taskbar with no menu
/// open is the stranding this module exists to recover from, and is
/// unaffected.
///
/// Restricted to the shell's own popups: the class belongs to XAML rather
/// than to the shell, so any application built on it would otherwise be
/// able to hold focus recovery off indefinitely.
#[cfg(target_os = "windows")]
pub(crate) fn shell_popup_is_open(dispatcher: &Dispatcher) -> bool {
  dispatcher.visible_windows().is_ok_and(|windows| {
    windows.iter().any(|window| {
      window
        .class_name()
        .is_ok_and(|class_name| class_name == SHELL_POPUP_WINDOW_CLASS)
        && window.process_name().is_ok_and(|process_name| {
          process_name.eq_ignore_ascii_case("explorer")
        })
        && popup_is_menu_sized(window)
    })
  })
}

/// Whether a shell popup is large enough to be a menu rather than a
/// tooltip.
///
/// Measured against the popup's owner, which for both is the taskbar. A
/// popup with no owner is not one of the shell's, and is left to the class
/// and process checks to reject.
#[cfg(target_os = "windows")]
fn popup_is_menu_sized(popup: &NativeWindow) -> bool {
  let Some(owner) = popup.owner_window() else {
    return false;
  };

  let (Ok(popup_frame), Ok(owner_frame)) = (popup.frame(), owner.frame())
  else {
    return false;
  };

  popup_frame.height()
    > owner_frame.height() * MENU_POPUP_MIN_TASKBAR_HEIGHTS
}

/// Whether a focus event target is the desktop or the taskbar frame.
#[cfg(target_os = "windows")]
pub(crate) fn is_shell_focus_target(native_window: &NativeWindow) -> bool {
  native_window.hwnd().0 == 0
    || native_window.is_desktop_window().unwrap_or(false)
    || native_window
      .class_name()
      .is_ok_and(|class_name| is_taskbar_window_class(&class_name))
}

#[cfg(any(test, target_os = "windows"))]
pub(crate) fn is_taskbar_window_class(class_name: &str) -> bool {
  matches!(class_name, "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

#[cfg(test)]
mod tests {
  use super::{
    is_taskbar_window_class, MENU_POPUP_MIN_TASKBAR_HEIGHTS,
    SHELL_POPUP_WINDOW_CLASS,
  };

  /// Heights measured from the shell against a 40px taskbar.
  const TASKBAR_HEIGHT: i32 = 40;
  const TOOLTIP_HEIGHT: i32 = 41;
  const SUBMENU_HEIGHT: i32 = 154;
  const MENU_HEIGHT: i32 = 608;

  fn is_menu_sized(popup_height: i32, taskbar_height: i32) -> bool {
    popup_height > taskbar_height * MENU_POPUP_MIN_TASKBAR_HEIGHTS
  }

  #[test]
  fn separates_the_shell_menus_from_its_tooltips() {
    assert!(is_menu_sized(MENU_HEIGHT, TASKBAR_HEIGHT));
    assert!(is_menu_sized(SUBMENU_HEIGHT, TASKBAR_HEIGHT));

    // The tooltip is a hair taller than the taskbar itself, and stays on
    // screen indefinitely once the shell strands one. Counting it as a
    // menu would hold focus recovery off for as long as it lasted.
    assert!(!is_menu_sized(TOOLTIP_HEIGHT, TASKBAR_HEIGHT));
  }

  #[test]
  fn the_separation_survives_display_scaling() {
    // Every height scales together, so the threshold has to hold at any
    // scaling rather than at the one it was measured on.
    for scale in [1, 2, 3] {
      assert!(is_menu_sized(MENU_HEIGHT * scale, TASKBAR_HEIGHT * scale));
      assert!(is_menu_sized(
        SUBMENU_HEIGHT * scale,
        TASKBAR_HEIGHT * scale
      ));
      assert!(!is_menu_sized(
        TOOLTIP_HEIGHT * scale,
        TASKBAR_HEIGHT * scale
      ));
    }
  }

  #[test]
  fn a_shell_popup_is_not_itself_a_taskbar() {
    // The two are read off different windows and mean opposite things: the
    // taskbar holding the foreground is what makes focus look stranded,
    // while a popup being on screen is what says it is not. Conflating
    // them would make an open Win+X menu its own justification for
    // stealing focus away from it.
    assert!(!is_taskbar_window_class(SHELL_POPUP_WINDOW_CLASS));
  }

  #[test]
  fn recognizes_only_taskbar_window_classes() {
    assert!(is_taskbar_window_class("Shell_TrayWnd"));
    assert!(is_taskbar_window_class("Shell_SecondaryTrayWnd"));
    assert!(!is_taskbar_window_class("Progman"));

    // The flyout hosts for Widgets, Search, and quick settings share this
    // class with the Start menu. Matching it would steal focus from a
    // flyout the user is actively typing into; the stranded-focus poll
    // recovers them after dismissal instead.
    assert!(!is_taskbar_window_class("Windows.UI.Core.CoreWindow"));
  }
}
