//! Recovery for a taskbar raised back over a fullscreen window.
//!
//! [`sync_fullscreen_mark`] tells the shell when a window covers the
//! taskbar, and the shell drops the taskbar behind it. Nothing holds it
//! there. A window *owned* by `Shell_TrayWnd` drags its owner to the top
//! of the z-order when it leaves top-most — which any taskbar overlay does
//! whenever it re-asserts its own z-order — and shell refreshes and
//! Explorer restarts raise it outright.
//!
//! None of that emits an event, and none of it changes anything the WM can
//! see: the window is still fullscreen, still on a displayed workspace,
//! and still marked. So `sync_fullscreen_mark` has nothing to react to and
//! would decline to re-mark even if it did, leaving the taskbar drawn over
//! the window until something unrelated changes. This polls for that state
//! instead of waiting for an event that never arrives.
//!
//! [`sync_fullscreen_mark`]: super::sync_fullscreen_mark

#[cfg(target_os = "windows")]
use tracing::info;
#[cfg(target_os = "windows")]
use wm_platform::{NativeWindow, NativeWindowWindowsExt};

#[cfg(target_os = "windows")]
use super::{
  platform_sync::is_covering_taskbar,
  reconcile_focus::is_taskbar_window_class,
};
#[cfg(target_os = "windows")]
use crate::traits::WindowGetters;
use crate::wm_state::WmState;

/// Windows examined above a fullscreen one before giving up.
///
/// The walk ends at the top of the z-order on its own, so this only bounds
/// the case where the stack is being reordered underneath it.
#[cfg(any(test, target_os = "windows"))]
const MAX_WINDOWS_ABOVE: usize = 500;

/// Re-marks fullscreen windows that the shell has raised the taskbar over.
///
/// A failed re-mark is logged rather than propagated, but the fallible
/// signature is kept so this matches the other arms of the main event
/// loop.
#[cfg(target_os = "windows")]
#[allow(clippy::unnecessary_wraps)]
pub fn reconcile_stranded_taskbar(state: &WmState) -> anyhow::Result<()> {
  // Nothing to hold down when the WM has not told the shell about
  // anything. This is the usual case, so the z-order walk below costs
  // nothing at all for as long as no window is covering the taskbar.
  if state.is_paused || state.windows_marked_fullscreen.is_empty() {
    return Ok(());
  }

  let stranded = state
    .windows()
    .into_iter()
    // Only put the taskbar back under a window that is still on screen and
    // still covering it. A mark the WM holds for a window that has stopped
    // covering the taskbar is stale, and belongs to `sync_fullscreen_mark`
    // to drop on the next redraw — forcing the taskbar down on its behalf
    // twice a second would leave the taskbar unusable until it did.
    .filter(|window| {
      state
        .windows_marked_fullscreen
        .contains(&window.native().id())
        && is_covering_taskbar(window)
    })
    .map(|window| window.native().clone())
    .filter(is_behind_taskbar)
    .collect::<Vec<_>>();

  for native in stranded {
    info!("Re-marking a fullscreen window the taskbar was raised over.");

    // The shell ignores a mark repeating what it already holds, so
    // asserting `true` again does nothing at all; the mark has to be
    // dropped first for the second call to be a change. The two are sent
    // back to back: measured against a taskbar raised over a fullscreen
    // window, the pair put it back behind the window before the next
    // frame, and any gap between them is a gap the taskbar is visible for.
    //
    // What the WM believes is unchanged throughout — the window was
    // fullscreen before this and still is — so `windows_marked_fullscreen`
    // keeps its entry and no event is emitted. Overlays listening for one
    // never see the window leave fullscreen, because it never did.
    if let Err(err) = native
      .mark_fullscreen(false)
      .and_then(|()| native.mark_fullscreen(true))
    {
      tracing::warn!("Failed to re-mark window as fullscreen: {err}");
    }
  }

  Ok(())
}

/// Gets whether the taskbar is drawn over the given window.
#[cfg(target_os = "windows")]
fn is_behind_taskbar(native: &NativeWindow) -> bool {
  // `window_above` walks towards the top of the stack, ending at the
  // window that has nothing over it.
  let above = std::iter::successors(
    native.window_above().ok().flatten(),
    |current| current.window_above().ok().flatten(),
  )
  .map(|window| {
    (
      window.class_name().unwrap_or_default(),
      window.is_visible().unwrap_or(false),
    )
  });

  has_taskbar_above(above)
}

/// Gets whether a visible taskbar appears among the windows drawn over
/// another one, given each of those as its class name and whether it is
/// visible, ordered upwards from the window in question.
///
/// A taskbar that is hidden or cloaked is covering nothing, so it does not
/// count: the shell cloaks the taskbar on a secondary monitor whose
/// workspace has been moved away, and that is not a taskbar to recover.
#[cfg(any(test, target_os = "windows"))]
fn has_taskbar_above(
  windows: impl Iterator<Item = (String, bool)>,
) -> bool {
  windows
    .take(MAX_WINDOWS_ABOVE)
    .any(|(class_name, is_visible)| {
      is_visible && is_taskbar_window_class(&class_name)
    })
}

#[cfg(not(target_os = "windows"))]
#[allow(clippy::unnecessary_wraps)]
pub fn reconcile_stranded_taskbar(_state: &WmState) -> anyhow::Result<()> {
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::{has_taskbar_above, MAX_WINDOWS_ABOVE};

  fn windows(
    entries: &[(&str, bool)],
  ) -> impl Iterator<Item = (String, bool)> {
    entries
      .iter()
      .map(|(class_name, is_visible)| {
        ((*class_name).to_string(), *is_visible)
      })
      .collect::<Vec<_>>()
      .into_iter()
  }

  #[test]
  fn a_taskbar_over_the_window_is_found() {
    assert!(has_taskbar_above(windows(&[
      ("Chrome_WidgetWin_1", true),
      ("Shell_TrayWnd", true),
    ])));

    assert!(has_taskbar_above(windows(&[(
      "Shell_SecondaryTrayWnd",
      true
    )])));
  }

  #[test]
  fn nothing_over_the_window_is_not_a_stranded_taskbar() {
    assert!(!has_taskbar_above(windows(&[])));
    assert!(!has_taskbar_above(windows(&[
      ("Chrome_WidgetWin_1", true),
      ("Progman", true),
    ])));
  }

  #[test]
  fn a_hidden_taskbar_is_covering_nothing() {
    assert!(!has_taskbar_above(windows(&[("Shell_TrayWnd", false)])));
  }

  #[test]
  fn the_walk_gives_up_rather_than_following_a_shifting_z_order() {
    // Stands in for a z-order being reordered while it is walked, which
    // would otherwise never reach the top of the stack.
    let endless =
      std::iter::repeat(("Chrome_WidgetWin_1".to_string(), true));
    assert!(!has_taskbar_above(endless));

    // The bound is a guard, not a search limit: a taskbar within reach of
    // it is still found.
    let buried = std::iter::repeat_n(
      ("Chrome_WidgetWin_1".to_string(), true),
      MAX_WINDOWS_ABOVE - 1,
    )
    .chain(std::iter::once(("Shell_TrayWnd".to_string(), true)));
    assert!(has_taskbar_above(buried));
  }
}
