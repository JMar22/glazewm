use wm_common::WindowState;

use super::set_focused_descendant;
use crate::{
  models::Container,
  traits::{CommonGetters, WindowGetters},
  wm_state::WmState,
};

/// Direction in which to cycle through windows.
#[derive(Clone, Copy)]
pub enum FocusCycleDirection {
  Next,
  Previous,
}

/// Focuses the next or previous window with the same state in the current
/// workspace.
///
/// Window order is based on the container tree and wraps at both ends.
pub fn focus_window_in_cycle(
  origin_container: &Container,
  direction: FocusCycleDirection,
  state: &mut WmState,
) {
  if let Some(focus_target) =
    window_cycle_target(origin_container, direction)
  {
    set_focused_descendant(&focus_target, None);
    state.pending_sync.queue_focus_change().queue_cursor_jump();
  }
}

/// Gets the next or previous window with the same state in the current
/// workspace.
///
/// Returns `None` when the origin is not a window or there is no other
/// window with the same state.
pub fn window_cycle_target(
  origin_container: &Container,
  direction: FocusCycleDirection,
) -> Option<Container> {
  let origin_window = origin_container.as_window_container().ok()?;
  let origin_state = origin_window.state();
  let workspace = origin_window.workspace()?;

  if matches!(origin_state, WindowState::Minimized) {
    return None;
  }

  let windows = workspace
    .descendants()
    .filter_map(|container| {
      let window = container.as_window_container().ok()?;
      window
        .state()
        .is_same_state(&origin_state)
        .then_some(container)
    })
    .collect::<Vec<_>>();

  if windows.len() < 2 {
    return None;
  }

  let origin_index = windows
    .iter()
    .position(|window| window.id() == origin_container.id())?;

  let target_index = match direction {
    FocusCycleDirection::Next => (origin_index + 1) % windows.len(),
    FocusCycleDirection::Previous => {
      origin_index.checked_sub(1).unwrap_or(windows.len() - 1)
    }
  };

  windows.get(target_index).cloned()
}

#[cfg(test)]
mod tests {
  use wm_common::{
    FloatingStateConfig, FullscreenStateConfig, WindowState,
  };

  use super::{window_cycle_target, FocusCycleDirection};
  use crate::{
    models::{NonTilingWindow, Workspace},
    traits::CommonGetters,
  };

  fn fullscreen_window() -> NonTilingWindow {
    NonTilingWindow::mock()
      .state(WindowState::Fullscreen(FullscreenStateConfig::default()))
      .call()
  }

  #[test]
  fn cycles_fullscreen_windows_and_wraps() {
    let first = fullscreen_window();
    let second = fullscreen_window();
    let third = fullscreen_window();

    Workspace::mock()
      .non_tiling_windows(vec![
        first.clone(),
        second.clone(),
        third.clone(),
      ])
      .call();

    let next = window_cycle_target(
      &third.clone().into(),
      FocusCycleDirection::Next,
    );
    let previous = window_cycle_target(
      &first.clone().into(),
      FocusCycleDirection::Previous,
    );

    assert_eq!(next.map(|window| window.id()), Some(first.id()));
    assert_eq!(previous.map(|window| window.id()), Some(third.id()));
  }

  #[test]
  fn skips_windows_with_a_different_state() {
    let first = fullscreen_window();
    let floating = NonTilingWindow::mock()
      .state(WindowState::Floating(FloatingStateConfig::default()))
      .call();
    let second = fullscreen_window();

    Workspace::mock()
      .non_tiling_windows(vec![first.clone(), floating, second.clone()])
      .call();

    let next = window_cycle_target(
      &first.clone().into(),
      FocusCycleDirection::Next,
    );

    assert_eq!(next.map(|window| window.id()), Some(second.id()));
  }

  #[test]
  fn does_nothing_without_another_window_in_the_same_state() {
    let fullscreen = fullscreen_window();
    let floating = NonTilingWindow::mock()
      .state(WindowState::Floating(FloatingStateConfig::default()))
      .call();

    Workspace::mock()
      .non_tiling_windows(vec![fullscreen.clone(), floating])
      .call();

    let target =
      window_cycle_target(&fullscreen.into(), FocusCycleDirection::Next);

    assert!(target.is_none());
  }
}
