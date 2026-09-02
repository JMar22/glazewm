use anyhow::Context;
#[cfg(target_os = "windows")]
use wm_common::WindowEffectConfig;
use wm_common::{
  CursorJumpTrigger, DisplayState, HideCorner, HideMethod, UniqueExt,
  WindowState, WmEvent,
};
#[cfg(target_os = "windows")]
use wm_platform::NativeWindowWindowsExt;
#[cfg(target_os = "windows")]
use wm_platform::{CornerStyle, Dispatcher, NativeWindow, OpacityValue};
use wm_platform::{Rect, WindowZOrder};

use crate::{
  models::{Container, WindowContainer},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn platform_sync(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let focused_container =
    state.focused_container().context("No focused container.")?;

  if state.pending_sync.needs_focus_update() {
    sync_focus(&focused_container, state, config)?;
  }

  if !state.pending_sync.containers_to_redraw().is_empty()
    || !state.pending_sync.workspaces_to_reorder().is_empty()
  {
    redraw_containers(&focused_container, state, config)?;
  }

  if state.pending_sync.needs_cursor_jump()
    && config.value.general.cursor_jump.enabled
  {
    jump_cursor(focused_container.clone(), state, config)?;
  }

  if state.pending_sync.needs_focused_effect_update()
    || state.pending_sync.needs_all_effects_update()
  {
    // Keep reference to the previous window that had focus effects
    // applied.
    let prev_effects_window = state.prev_effects_window.clone();

    if let Ok(window) = focused_container.as_window_container() {
      apply_window_effects(&window, true, config);
      state.prev_effects_window = Some(window.clone());
    } else {
      state.prev_effects_window = None;
    }

    // Get windows that should have the unfocused border applied to them.
    // For the sake of performance, we only update the border of the
    // previously focused window. If the `reset_window_effects` flag is
    // passed, the unfocused border is applied to all unfocused windows.
    let unfocused_windows =
      if state.pending_sync.needs_all_effects_update() {
        state.windows()
      } else {
        prev_effects_window.into_iter().collect()
      }
      .into_iter()
      .filter(|window| window.id() != focused_container.id());

    for window in unfocused_windows {
      apply_window_effects(&window, false, config);
    }
  }

  state.pending_sync.clear();

  Ok(())
}

fn sync_focus(
  focused_container: &Container,
  state: &mut WmState,
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  config: &UserConfig,
) -> anyhow::Result<()> {
  let native_window = focused_container.as_window_container().ok();

  #[cfg(target_os = "windows")]
  let previous_foreground = state.dispatcher.focused_window().ok();

  // Sets focus to the appropriate target:
  // - If the container is a window, focuses that window.
  // - If the container is a workspace, "resets" focus by focusing the
  //   desktop window.
  //
  // In either case, a `PlatformEvent::WindowFocused` event is subsequently
  // triggered.
  let result = if let Some(window) = &native_window {
    tracing::info!("Setting focus to window: {window}");
    window.native().focus()
  } else {
    tracing::info!("Setting focus to the desktop window.");
    state.dispatcher.reset_focus()
  };

  if let Err(err) = result {
    tracing::warn!("Failed to set focus: {}", err);
  }

  // SetForegroundWindow can be denied transiently by Windows even after
  // the foreground-input workaround in NativeWindow::focus. Verify the
  // result after the command has unwound and retry while focus remains
  // on the exact window that preceded it. If the user, a dialog, or a
  // newly launched app focuses anything else, the retry stops without
  // stealing focus back.
  #[cfg(target_os = "windows")]
  if config.value.general.restore_focus_on_shell {
    if let (Some(window), Some(previous_foreground)) =
      (native_window, previous_foreground)
    {
      schedule_focus_convergence(
        window.native().clone(),
        previous_foreground,
        &state.dispatcher,
      );
    }
  }

  state.emit_event(WmEvent::FocusChanged {
    focused_container: focused_container.to_dto()?,
  });

  Ok(())
}

#[cfg(target_os = "windows")]
const FOCUS_CONVERGENCE_DELAYS_MS: [u64; 4] = [75, 150, 300, 600];

#[cfg(target_os = "windows")]
fn schedule_focus_convergence(
  target: NativeWindow,
  previous_foreground: NativeWindow,
  dispatcher: &Dispatcher,
) {
  let dispatcher = dispatcher.clone();

  tokio::task::spawn(async move {
    for delay_ms in FOCUS_CONVERGENCE_DELAYS_MS {
      tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

      let Ok(current) = dispatcher.focused_window() else {
        return;
      };

      if current == target {
        return;
      }

      if current != previous_foreground
        || !target.is_valid()
        || !target.is_visible().unwrap_or(false)
        || target.is_minimized().unwrap_or(true)
      {
        return;
      }

      tracing::info!(
        "Retrying focus while Windows foreground remains unchanged."
      );
      if let Err(err) = target.focus() {
        tracing::warn!("Failed to retry focus: {err}");
      }
    }
  });
}

/// Finds windows that should be brought to the top of their workspace's
/// z-order.
///
/// Windows are brought to front if they match the focused window's state
/// (floating/tiling) and any of these conditions are met:
///  * Focus has changed to a different window.
///  * Focused window's state has changed (e.g. tiling -> floating).
///  * Focused window has moved to a different workspace.
fn windows_to_bring_to_front(
  focused_container: &Container,
  state: &WmState,
) -> anyhow::Result<Vec<WindowContainer>> {
  let focused_workspace =
    focused_container.workspace().context("No workspace.")?;

  // Add focused workspace if there's been a focus change.
  let workspaces_to_reorder = state
    .pending_sync
    .workspaces_to_reorder()
    .iter()
    .chain(
      state
        .pending_sync
        .needs_focus_update()
        .then_some(&focused_workspace),
    )
    .unique_by(|workspace| workspace.id());

  // Bring forward windows that match the focused state. Only do this for
  // tiling/floating windows.
  let windows_to_bring_to_front = workspaces_to_reorder
    .flat_map(|workspace| {
      let focused_descendant = workspace
        .descendant_focus_order()
        .next()
        .and_then(|container| container.as_window_container().ok());

      match focused_descendant {
        Some(focused_descendant) => workspace
          .descendants()
          .filter_map(|descendant| descendant.as_window_container().ok())
          .filter(|window| {
            let is_floating_or_tiling = matches!(
              window.state(),
              WindowState::Floating(_) | WindowState::Tiling
            );

            is_floating_or_tiling
              && window.state().is_same_state(&focused_descendant.state())
          })
          .collect(),
        None => vec![],
      }
    })
    .collect::<Vec<_>>();

  Ok(windows_to_bring_to_front)
}

#[allow(clippy::too_many_lines)]
fn redraw_containers(
  focused_container: &Container,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let windows_to_redraw = state.windows_to_redraw();
  let windows_to_bring_to_front =
    windows_to_bring_to_front(focused_container, state)?;

  let windows_to_update = {
    let mut windows = windows_to_redraw
      .iter()
      .chain(&windows_to_bring_to_front)
      .unique_by(|window| window.id())
      .collect::<Vec<_>>();

    let descendant_focus_order = state
      .root_container
      .descendant_focus_order()
      .collect::<Vec<_>>();

    // The update loop below runs this list in reverse. Group windows from
    // displayed workspaces at the end so they are revealed before windows
    // from the outgoing workspace are hidden. This prevents the desktop
    // from flashing between the two operations. Within each group, retain
    // focus order so the most recently focused window is updated last and
    // remains at the top of the stack.
    windows.sort_by_key(|window| {
      redraw_sort_key(
        window
          .workspace()
          .is_some_and(|workspace| workspace.is_displayed()),
        descendant_focus_order
          .iter()
          .position(|order| order.id() == window.id()),
      )
    });

    windows
  };

  // Get monitors by their optimal hide corner.
  let monitors_by_hide_corner = state.monitors_by_hide_corner();

  for window in windows_to_update.iter().rev() {
    let should_bring_to_front = windows_to_bring_to_front.contains(window);

    let workspace =
      window.workspace().context("Window has no workspace.")?;

    let monitor = window.monitor().context("No monitor.")?;
    let hide_corner = monitors_by_hide_corner
      .iter()
      .find(|(m, _)| m.id() == monitor.id())
      .map(|(_, hide_corner)| hide_corner)
      .context("Monitor not found in hide corner map.")?;

    // Whether the window should be shown above all other windows.
    let z_order = match window.state() {
      WindowState::Floating(config) if config.shown_on_top => {
        WindowZOrder::TopMost
      }
      WindowState::Fullscreen(config) if config.shown_on_top => {
        WindowZOrder::TopMost
      }
      _ if should_bring_to_front => {
        let focused_descendant = workspace
          .descendant_focus_order()
          .next()
          .and_then(|container| container.as_window_container().ok());

        if let Some(focused_descendant) = focused_descendant {
          if window.id() == focused_descendant.id() {
            WindowZOrder::Normal
          } else {
            WindowZOrder::AfterWindow(focused_descendant.native().id())
          }
        } else {
          WindowZOrder::Normal
        }
      }
      _ => WindowZOrder::Normal,
    };

    // Set the z-order of the window.
    //
    // NOTE: macOS doesn't have a robust public API for setting the z-order
    // of a window. See `NativeWindow::raise` for more details.
    #[cfg(target_os = "windows")]
    if should_bring_to_front && !windows_to_redraw.contains(window) {
      tracing::info!("Updating window z-order: {window}");

      if let Err(err) = window.native().set_z_order(&z_order) {
        tracing::warn!("Failed to set window z-order: {}", err);
      }
    }

    // Skip updating the window's position if it only required a z-order
    // change.
    if !windows_to_redraw.contains(window) {
      continue;
    }

    // Transition display state depending on whether window will be
    // shown or hidden.
    window.set_display_state(
      match (window.display_state(), workspace.is_displayed()) {
        (DisplayState::Hidden | DisplayState::Hiding, true) => {
          DisplayState::Showing
        }
        (DisplayState::Shown | DisplayState::Showing, false) => {
          DisplayState::Hiding
        }
        _ => window.display_state(),
      },
    );

    let is_visible = matches!(
      window.display_state(),
      DisplayState::Showing | DisplayState::Shown
    );

    if let Err(err) =
      reposition_window(window, *hide_corner, &z_order, is_visible, config)
    {
      tracing::warn!("Failed to set window position: {}", err);
    }

    // Tell the shell whether the window is covering the taskbar, so that
    // it drops the taskbar below the window and raises it again
    // afterwards.
    #[cfg(target_os = "windows")]
    sync_fullscreen_mark(window, state);

    // Reassert removal for every redraw of a hidden cloaked window. Some
    // applications can add their taskbar tab again after GlazeWM's
    // initial Hiding transition. Visible windows are added only during
    // Showing so a general redraw doesn't reorder taskbar tabs.
    #[cfg(target_os = "windows")]
    if should_sync_taskbar_visibility(
      &config.value.general.hide_method,
      config.value.general.show_all_in_taskbar,
      &window.display_state(),
      is_visible,
    ) {
      if let Err(err) = window.native().set_taskbar_visibility(is_visible)
      {
        tracing::warn!("Failed to set taskbar visibility: {}", err);
      }
    }
  }

  Ok(())
}

fn redraw_sort_key(
  will_be_visible: bool,
  focus_order: Option<usize>,
) -> (bool, Option<usize>) {
  (will_be_visible, focus_order)
}

/// Gets whether a window's frame is drawn over the taskbar.
///
/// This is a question about geometry, not about window state. An
/// application entering its own fullscreen from a maximized window keeps
/// the native maximized flag — Firefox does on F11 — so the WM goes on
/// holding it in a *maximized* fullscreen while its frame grows from the
/// working area to the whole monitor. The taskbar lives in the difference
/// between those two rects, so that is what gets measured.
#[cfg(any(test, target_os = "windows"))]
fn covers_taskbar(
  frame: &Rect,
  bounds: &Rect,
  working_area: &Rect,
) -> bool {
  // A monitor whose working area is its full bounds has no taskbar strip
  // to cover: it is on another monitor, or set to auto-hide.
  bounds != working_area
    // Inset so that a frame matching the monitor exactly still counts.
    && frame.contains_rect(&bounds.inset(1))
}

/// Gets whether a window has already placed itself over the whole monitor.
///
/// An application in its own fullscreen has put its frame exactly where
/// the WM would put it. Re-applying the WM's own rect to such a window is
/// not the no-op it looks like: the rect carries the invisible resize
/// border back from the window's cached shadow borders, which the
/// application dropped on the way into fullscreen. Measured on a
/// fullscreen video, a frame of `0,0,2880,1800` came back from a workspace
/// switch as `-11,0,2891,1811` — off the monitor's origin and past its far
/// edge, so what the window draws is shifted and cropped by the border
/// width.
///
/// Nothing about such a window needs moving, so nothing is moved.
#[cfg(any(test, target_os = "windows"))]
fn fills_monitor(frame: &Rect, bounds: &Rect) -> bool {
  // Inset so that a frame matching the monitor exactly still counts.
  frame.contains_rect(&bounds.inset(1))
}

/// Gets whether a window is one the WM should leave where it is: it is in
/// a fullscreen state and has already taken its whole monitor.
#[cfg(target_os = "windows")]
fn keeps_own_fullscreen_frame(window: &WindowContainer) -> bool {
  matches!(window.state(), WindowState::Fullscreen(_))
    && window.monitor().is_some_and(|monitor| {
      fills_monitor(
        &window.native_properties().frame,
        &monitor.native_properties().bounds,
      )
    })
}

/// Gets whether a window is on screen and drawn over its monitor's
/// taskbar.
///
/// This is what the shell is told, and so also what has to still hold
/// before a lost mark is worth re-asserting.
#[cfg(target_os = "windows")]
pub(crate) fn is_covering_taskbar(window: &WindowContainer) -> bool {
  let is_visible = matches!(
    window.display_state(),
    DisplayState::Showing | DisplayState::Shown
  );

  is_visible
    && window.monitor().is_some_and(|monitor| {
      let monitor_properties = monitor.native_properties();

      covers_taskbar(
        &window.native_properties().frame,
        &monitor_properties.bounds,
        &monitor_properties.working_area,
      )
    })
}

/// Marks a window as fullscreen with the shell, or unmarks it, whenever
/// that differs from what the shell was last told.
///
/// A window cloaked on a hidden workspace keeps its fullscreen frame, but
/// covers nothing, so the mark is dropped as it hides and re-asserted as
/// it shows. Re-asserting matters: the shell ignores a mark that repeats
/// what it already holds, so without the drop, switching back to the
/// workspace of a fullscreen video left the taskbar drawn over it.
///
/// Called both when a window is redrawn, which is what changes whether it
/// is on screen, and when one is moved or resized, which is what changes
/// whether it covers the taskbar. An application entering its own
/// fullscreen only does the latter: the WM's state for the window comes
/// out of it unchanged, so no redraw follows.
#[cfg(target_os = "windows")]
pub(crate) fn sync_fullscreen_mark(
  window: &WindowContainer,
  state: &mut WmState,
) {
  let window_id = window.native().id();
  let is_fullscreen = is_covering_taskbar(window);

  if is_fullscreen == state.windows_marked_fullscreen.contains(&window_id)
  {
    return;
  }

  if let Err(err) = window.native().mark_fullscreen(is_fullscreen) {
    tracing::warn!("Failed to mark window as fullscreen: {}", err);
    return;
  }

  if is_fullscreen {
    state.windows_marked_fullscreen.insert(window_id);
  } else {
    state.windows_marked_fullscreen.remove(&window_id);
  }

  state.emit_event(WmEvent::FullscreenChanged {
    fullscreen_id: window.id(),
    is_fullscreen,
  });
}

#[cfg(any(test, target_os = "windows"))]
fn should_sync_taskbar_visibility(
  hide_method: &HideMethod,
  show_all_in_taskbar: bool,
  display_state: &DisplayState,
  is_visible: bool,
) -> bool {
  *hide_method == HideMethod::Cloak
    && !show_all_in_taskbar
    && (!is_visible || *display_state == DisplayState::Showing)
}

fn reposition_window(
  window: &WindowContainer,
  hide_corner: HideCorner,
  // LINT: `z_order` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  z_order: &WindowZOrder,
  is_visible: bool,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let rect = window
    .to_rect()?
    .apply_delta(&window.total_border_delta()?, None);

  // For `HideMethod::PlaceInCorner`, we need to reposition hidden windows
  // to the corner of the monitor.
  if config.value.general.hide_method == HideMethod::PlaceInCorner
    && !is_visible
  {
    const VISIBLE_SLIVER: i32 = 1;

    let monitor_rect = window
      .monitor()
      .context("No monitor.")?
      .native_properties()
      .working_area;

    let frame = window.native_properties().frame;

    let position_y = monitor_rect.bottom - VISIBLE_SLIVER;
    let position_x = match hide_corner {
      HideCorner::BottomLeft => {
        monitor_rect.left + VISIBLE_SLIVER - frame.width()
      }
      HideCorner::BottomRight => monitor_rect.right - VISIBLE_SLIVER,
    };

    // Even though the window size is unchanged, `NativeWindow::set_frame`
    // is used instead of `NativeWindow::reposition` because the latter
    // resulted in occasional incorrect positionings on macOS.
    window.native().set_frame(&Rect::from_xy(
      position_x,
      position_y,
      frame.width(),
      frame.height(),
    ))?;

    return Ok(());
  }

  if window.active_drag().is_some() {
    window.native().resize(rect.width(), rect.height())?;
  } else {
    #[cfg(target_os = "macos")]
    window.native().set_frame(&rect)?;

    #[cfg(target_os = "windows")]
    {
      use wm_platform::{
        SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSENDCHANGING, SWP_NOSIZE,
        WS_MAXIMIZEBOX,
      };

      let keeps_own_frame = keeps_own_fullscreen_frame(window);

      // Restore window if it's minimized/maximized and shouldn't be. This
      // is needed to be able to move and resize it.
      let should_restore = !keeps_own_frame
        && match &window.state() {
          // Need to restore window if transitioning from maximized
          // fullscreen to non-maximized fullscreen.
          WindowState::Fullscreen(fullscreen) => {
            !fullscreen.maximized && window.native().is_maximized()?
          }
          // No need to restore window if it'll be minimized. Transitioning
          // from maximized to minimized works without having to
          // restore.
          WindowState::Minimized => false,
          _ => {
            window.native().is_minimized()?
              || window.native().is_maximized()?
          }
        };

      if should_restore {
        // Restoring to position has the same effect as `ShowWindow` with
        // `SW_RESTORE`, but doesn't cause a flicker.
        window.native().restore(Some(&rect))?;
      }

      let mut swp_flags = SWP_NOACTIVATE
        | SWP_NOCOPYBITS
        | SWP_NOSENDCHANGING
        | SWP_ASYNCWINDOWPOS;

      // The call still runs, for its z-order half.
      if keeps_own_frame {
        swp_flags |= SWP_NOMOVE | SWP_NOSIZE;
      }

      match &window.state() {
        WindowState::Minimized => {
          if !window.native().is_minimized()? {
            window.native().minimize()?;
          }
        }
        WindowState::Fullscreen(fullscreen)
          if fullscreen.maximized
            && window.native().has_window_style(WS_MAXIMIZEBOX) =>
        {
          // Maximizing a window that is drawing over the whole monitor on
          // its own terms would take it back out of that.
          if !keeps_own_frame && !window.native().is_maximized()? {
            window.native().maximize()?;
          }

          window.native().set_window_pos(z_order, &rect, swp_flags)?;
        }
        _ => {
          if !keeps_own_frame {
            swp_flags |= SWP_FRAMECHANGED;
          }

          window.native().set_window_pos(z_order, &rect, swp_flags)?;

          // When there's a mismatch between the DPI of the monitor and the
          // window, the window might be sized incorrectly after the first
          // move. If we set the position twice, inconsistencies after the
          // first move are resolved.
          if window.has_pending_dpi_adjustment() {
            window.native().set_window_pos(z_order, &rect, swp_flags)?;
          }
        }
      }

      // Set visibility based on the hide method.
      if config.value.general.hide_method == HideMethod::Cloak {
        window.native().set_cloaked(!is_visible)?;
      } else if is_visible {
        window.native().show()?;
      } else {
        window.native().hide()?;
      }
    }
  }

  Ok(())
}

fn jump_cursor(
  focused_container: Container,
  state: &WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let cursor_jump = &config.value.general.cursor_jump;

  let jump_target = match cursor_jump.trigger {
    CursorJumpTrigger::WindowFocus => Some(focused_container),
    CursorJumpTrigger::MonitorFocus => {
      let target_monitor =
        focused_container.monitor().context("No monitor.")?;

      let cursor_monitor = state
        .dispatcher
        .cursor_position()
        .ok()
        .and_then(|pos| state.monitor_at_point(&pos));

      // Jump to the target monitor if the cursor is not already on it.
      cursor_monitor
        .filter(|monitor| monitor.id() != target_monitor.id())
        .map(|_| target_monitor.into())
    }
  };

  if let Some(jump_target) = jump_target {
    let center = jump_target.to_rect()?.center_point();

    if let Err(err) = state.dispatcher.set_cursor_position(&center) {
      tracing::warn!("Failed to set cursor position: {}", err);
    }
  }

  Ok(())
}

fn apply_window_effects(
  // LINT: `window` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  window: &WindowContainer,
  is_focused: bool,
  config: &UserConfig,
) {
  let window_effects = &config.value.window_effects;

  // LINT: `effect_config` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  let effect_config = if is_focused {
    &window_effects.focused_window
  } else {
    &window_effects.other_windows
  };

  // Skip if both focused + non-focused window effects are disabled.
  #[cfg(target_os = "windows")]
  if window_effects.focused_window.border.enabled
    || window_effects.other_windows.border.enabled
  {
    apply_border_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.hide_title_bar.enabled
    || window_effects.other_windows.hide_title_bar.enabled
  {
    apply_hide_title_bar_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.corner_style.enabled
    || window_effects.other_windows.corner_style.enabled
  {
    apply_corner_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.transparency.enabled
    || window_effects.other_windows.transparency.enabled
  {
    apply_transparency_effect(window, effect_config);
  }
}

#[cfg(target_os = "windows")]
fn apply_border_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let border_color = if effect_config.border.enabled {
    Some(&effect_config.border.color)
  } else {
    None
  };

  _ = window.native().set_border_color(border_color);

  let native = window.native().clone();
  let border_color = border_color.cloned();

  // Re-apply border color after a short delay to better handle
  // windows that change it themselves.
  tokio::task::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    _ = native.set_border_color(border_color.as_ref());
  });
}

#[cfg(target_os = "windows")]
fn apply_hide_title_bar_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  _ = window
    .native()
    .set_title_bar_visibility(!effect_config.hide_title_bar.enabled);
}

#[cfg(target_os = "windows")]
fn apply_corner_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let corner_style = if effect_config.corner_style.enabled {
    &effect_config.corner_style.style
  } else {
    &CornerStyle::Default
  };

  _ = window.native().set_corner_style(corner_style);
}

#[cfg(target_os = "windows")]
fn apply_transparency_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let transparency = if effect_config.transparency.enabled {
    &effect_config.transparency.opacity
  } else {
    // Reset the transparency to default.
    &OpacityValue::from_alpha(u8::MAX)
  };

  _ = window.native().set_transparency(transparency);
}

#[cfg(test)]
mod tests {
  use wm_common::{DisplayState, HideMethod};
  use wm_platform::Rect;

  use super::{
    covers_taskbar, fills_monitor, redraw_sort_key,
    should_sync_taskbar_visibility,
  };

  #[test]
  fn redraws_destination_before_hiding_outgoing_workspace() {
    let mut windows = [
      ("outgoing_focused", false, Some(0)),
      ("destination_focused", true, Some(1)),
      ("outgoing_other", false, Some(2)),
      ("destination_other", true, Some(3)),
    ];

    windows.sort_by_key(|(_, is_displayed, focus_order)| {
      redraw_sort_key(*is_displayed, *focus_order)
    });

    let update_order = windows
      .iter()
      .rev()
      .map(|(name, _, _)| *name)
      .collect::<Vec<_>>();

    assert_eq!(
      update_order,
      [
        "destination_other",
        "destination_focused",
        "outgoing_other",
        "outgoing_focused",
      ]
    );
  }

  /// Width of the invisible resize border Windows lays outside a window's
  /// visible frame. The real value follows the monitor's DPI; what
  /// matters to the check is that a maximized frame is grown by it on
  /// every side, which is why such a frame overshoots the monitor on the
  /// free sides while still stopping at the strip the taskbar reserves.
  const RESIZE_BORDER: i32 = 13;

  /// Where Windows puts a window maximized on a monitor with this working
  /// area.
  fn maximized_frame(working_area: &Rect) -> Rect {
    working_area.inset(-RESIZE_BORDER)
  }

  /// Monitors to check the rule against, as (bounds, working area).
  ///
  /// Between them they vary everything the rule must not depend on: which
  /// edge reserves a strip for the taskbar, how thick it is, the
  /// monitor's resolution, and whether its origin is (0, 0).
  fn monitors() -> [(Rect, Rect); 6] {
    [
      // Taskbar along the bottom, the usual arrangement.
      (
        Rect::from_ltrb(0, 0, 1920, 1080),
        Rect::from_ltrb(0, 0, 1920, 1032),
      ),
      // The same edge on a larger monitor with a thicker strip.
      (
        Rect::from_ltrb(0, 0, 2560, 1440),
        Rect::from_ltrb(0, 0, 2560, 1380),
      ),
      // The other three edges.
      (
        Rect::from_ltrb(0, 0, 1920, 1080),
        Rect::from_ltrb(0, 60, 1920, 1080),
      ),
      (
        Rect::from_ltrb(0, 0, 1920, 1080),
        Rect::from_ltrb(96, 0, 1920, 1080),
      ),
      (
        Rect::from_ltrb(0, 0, 3840, 2160),
        Rect::from_ltrb(0, 0, 3720, 2160),
      ),
      // A monitor left of and above the primary one, so that its bounds
      // are negative on both axes.
      (
        Rect::from_ltrb(-1920, -120, 0, 960),
        Rect::from_ltrb(-1920, -120, 0, 912),
      ),
    ]
  }

  #[test]
  fn a_frame_at_the_monitor_bounds_covers_the_taskbar() {
    for (bounds, working_area) in monitors() {
      assert!(
        covers_taskbar(&bounds, &bounds, &working_area),
        "frame at the monitor's own bounds: {bounds:?}",
      );

      // An application that keeps its resize borders on the way into
      // fullscreen overshoots the monitor, which covers it just as well.
      assert!(
        covers_taskbar(
          &bounds.inset(-RESIZE_BORDER),
          &bounds,
          &working_area
        ),
        "frame past the monitor's bounds: {bounds:?}",
      );
    }
  }

  #[test]
  fn a_maximized_frame_does_not_cover_the_taskbar() {
    // Past the monitor on the free sides, stopping at the reserved strip.
    // This is the case a state-based check got wrong: the WM holds such a
    // window in a maximized fullscreen either way, so only the frame
    // tells the two apart.
    for (bounds, working_area) in monitors() {
      assert!(
        !covers_taskbar(
          &maximized_frame(&working_area),
          &bounds,
          &working_area
        ),
        "maximized frame on {bounds:?}",
      );

      assert!(
        !covers_taskbar(&working_area, &bounds, &working_area),
        "frame filling the working area of {bounds:?}",
      );
    }
  }

  #[test]
  fn nothing_covers_a_taskbar_that_reserves_no_space() {
    // An auto-hidden taskbar leaves the whole monitor as working area, so
    // a window covering the monitor covers nothing that was on screen.
    for (bounds, _) in monitors() {
      assert!(
        !covers_taskbar(&bounds, &bounds, &bounds),
        "monitor with no reserved strip: {bounds:?}",
      );
    }
  }

  #[test]
  fn a_window_over_the_whole_monitor_keeps_its_own_frame() {
    for (bounds, _) in monitors() {
      assert!(
        fills_monitor(&bounds, &bounds),
        "frame at the monitor's own bounds: {bounds:?}",
      );

      // What the WM would otherwise re-apply: its rect grown by the
      // window's cached resize border, which is where the drift came from.
      assert!(
        fills_monitor(&bounds.inset(-RESIZE_BORDER), &bounds),
        "frame past the monitor's bounds: {bounds:?}",
      );
    }
  }

  #[test]
  fn a_window_short_of_the_monitor_is_still_placed_by_the_wm() {
    for (bounds, working_area) in monitors() {
      // A maximized window stops at the taskbar, so the WM still owns it.
      assert!(
        !fills_monitor(&maximized_frame(&working_area), &bounds),
        "maximized frame on {bounds:?}",
      );

      assert!(
        !fills_monitor(&working_area, &bounds),
        "frame filling the working area of {bounds:?}",
      );

      // A window fullscreen on a different monitor does not count as
      // having placed itself on this one.
      let elsewhere = Rect::from_ltrb(
        bounds.left + bounds.width(),
        bounds.top,
        bounds.right + bounds.width(),
        bounds.bottom,
      );

      assert!(
        !fills_monitor(&elsewhere, &bounds),
        "frame on the neighbouring monitor of {bounds:?}",
      );
    }
  }

  #[test]
  fn syncs_hidden_cloaked_windows_on_every_redraw() {
    assert!(should_sync_taskbar_visibility(
      &HideMethod::Cloak,
      false,
      &DisplayState::Hidden,
      false,
    ));
    assert!(should_sync_taskbar_visibility(
      &HideMethod::Cloak,
      false,
      &DisplayState::Hiding,
      false,
    ));
  }

  #[test]
  fn adds_visible_taskbar_tabs_only_while_showing() {
    assert!(should_sync_taskbar_visibility(
      &HideMethod::Cloak,
      false,
      &DisplayState::Showing,
      true,
    ));
    assert!(!should_sync_taskbar_visibility(
      &HideMethod::Cloak,
      false,
      &DisplayState::Shown,
      true,
    ));
  }

  #[test]
  fn respects_taskbar_and_hide_method_configuration() {
    assert!(!should_sync_taskbar_visibility(
      &HideMethod::Cloak,
      true,
      &DisplayState::Hidden,
      false,
    ));
    assert!(!should_sync_taskbar_visibility(
      &HideMethod::Hide,
      false,
      &DisplayState::Hidden,
      false,
    ));
  }
}
