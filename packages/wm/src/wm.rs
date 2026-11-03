use anyhow::{bail, Context};
use tokio::sync::mpsc::{self};
use tracing::warn;
use uuid::Uuid;
#[cfg(target_os = "windows")]
use wm_common::TitleBarVisibility;
use wm_common::{
  FloatingStateConfig, FullscreenStateConfig, InvokeCommand,
  InvokeFocusCommand, InvokeMoveCommand, WindowState, WmEvent,
};
#[cfg(target_os = "windows")]
use wm_platform::NativeWindowWindowsExt;
use wm_platform::{
  Dispatcher, LengthValue, PlatformEvent, RectDelta, WindowEvent,
};

use crate::{
  commands::{
    container::{
      focus_container_by_id, focus_in_direction, focus_window_in_cycle,
      set_tiling_direction, toggle_tiling_direction, FocusCycleDirection,
    },
    general::{
      cycle_focus, disable_binding_mode, enable_binding_mode,
      platform_sync, reload_config, shell_exec, toggle_pause,
    },
    monitor::focus_monitor,
    window::{
      ignore_window, move_window_in_direction, move_window_to_workspace,
      resize_window, set_window_position, set_window_size,
      update_window_state, WindowPositionTarget,
    },
    workspace::{
      focus_workspace, move_workspace_in_direction,
      update_workspace_config,
    },
  },
  events::{
    handle_display_settings_changed, handle_mouse_move,
    handle_window_destroyed, handle_window_focused, handle_window_hidden,
    handle_window_minimize_ended, handle_window_minimized,
    handle_window_moved_or_resized, handle_window_shown,
    handle_window_title_changed,
  },
  ipc_server::IpcServer,
  models::{Container, WindowContainer, WorkspaceTarget},
  traits::{CommonGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Placement options of a `set-floating` command, each unset unless given.
struct FloatingPlacement {
  centered: Option<bool>,
  shown_on_top: Option<bool>,
  position: (Option<i32>, Option<i32>),
  size: (Option<LengthValue>, Option<LengthValue>),
}

pub struct WindowManager {
  pub event_rx: mpsc::UnboundedReceiver<WmEvent>,
  pub exit_rx: mpsc::UnboundedReceiver<()>,
  pub state: WmState,
}

impl WindowManager {
  pub fn new(
    config: &mut UserConfig,
    dispatcher: Dispatcher,
  ) -> anyhow::Result<Self> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (exit_tx, exit_rx) = mpsc::unbounded_channel();

    let mut state = WmState::new(dispatcher, event_tx, exit_tx);
    state.populate(config)?;

    Ok(Self {
      event_rx,
      exit_rx,
      state,
    })
  }

  pub fn process_event(
    &mut self,
    event: PlatformEvent,
    config: &mut UserConfig,
  ) -> anyhow::Result<()> {
    let state = &mut self.state;

    match event {
      PlatformEvent::DisplaySettingsChanged => {
        handle_display_settings_changed(state, config)
      }
      PlatformEvent::Keybinding(keybinding_event) => {
        // Find the keybinding config that matches this keybinding.
        let commands = config
          .active_keybinding_configs(
            &self.state.binding_modes,
            self.state.is_paused,
          )
          .find(|kb_config| {
            kb_config.bindings.contains(&keybinding_event.0)
          })
          .map(|kb_config| kb_config.commands.clone());

        if let Some(commands) = commands {
          self.process_commands(&commands, None, config)?;
        }

        // Return early since we don't want to redraw twice.
        return Ok(());
      }
      PlatformEvent::Mouse(event) => {
        handle_mouse_move(&event, state, config)
      }
      PlatformEvent::Window(window_event) => match window_event {
        WindowEvent::Focused { window, .. } => {
          handle_window_focused(&window, state, config)
        }
        WindowEvent::Shown { window, .. } => {
          handle_window_shown(window, state, config)
        }
        WindowEvent::Hidden { window, .. } => {
          handle_window_hidden(&window, state, config)
        }
        WindowEvent::MovedOrResized {
          window,
          is_interactive_start,
          is_interactive_end,
          ..
        } => handle_window_moved_or_resized(
          &window,
          is_interactive_start,
          is_interactive_end,
          state,
          config,
        ),
        WindowEvent::Minimized { window, .. } => {
          handle_window_minimized(&window, state, config)
        }
        WindowEvent::MinimizeEnded { window, .. } => {
          handle_window_minimize_ended(&window, state, config)
        }
        WindowEvent::TitleChanged { window, .. } => {
          handle_window_title_changed(&window, state, config)
        }
        WindowEvent::Destroyed { window_id, .. } => {
          handle_window_destroyed(window_id, state)
        }
      },
    }?;

    if !state.is_paused && state.pending_sync.has_changes() {
      platform_sync(state, config)?;
    }

    Ok(())
  }

  pub fn process_commands(
    &mut self,
    commands: &Vec<InvokeCommand>,
    subject_container_id: Option<Uuid>,
    config: &mut UserConfig,
  ) -> anyhow::Result<Uuid> {
    let state = &mut self.state;

    // Get the container to run WM commands with.
    let subject_container = match subject_container_id {
      Some(id) => state.container_by_id(id).with_context(|| {
        format!("No container found with the given ID '{id}'.")
      })?,
      None => state
        .focused_container()
        .context("No subject container for command.")?,
    };

    let new_subject_container_id = WindowManager::run_commands(
      commands,
      subject_container,
      state,
      config,
    )?;

    if state.pending_sync.has_changes() {
      platform_sync(state, config)?;
    }

    Ok(new_subject_container_id)
  }

  pub fn run_commands(
    commands: &Vec<InvokeCommand>,
    subject_container: Container,
    state: &mut WmState,
    config: &mut UserConfig,
  ) -> anyhow::Result<Uuid> {
    let mut current_subject_container = subject_container;

    for command in commands {
      WindowManager::run_command(
        command,
        current_subject_container.clone(),
        state,
        config,
      )?;

      // Update the subject container in case the container type changes.
      // For example, when going from a tiling to a floating window.
      current_subject_container =
        if current_subject_container.is_detached() {
          match state.container_by_id(current_subject_container.id()) {
            Some(container) => container,
            None => break,
          }
        } else {
          current_subject_container
        }
    }

    Ok(current_subject_container.id())
  }

  pub fn run_command(
    command: &InvokeCommand,
    subject_container: Container,
    state: &mut WmState,
    config: &mut UserConfig,
  ) -> anyhow::Result<()> {
    // No-op if WM is currently paused.
    if state.is_paused && *command != InvokeCommand::WmTogglePause {
      return Ok(());
    }

    if subject_container.is_detached() {
      bail!("Cannot run command because subject container is detached.");
    }

    match command {
      InvokeCommand::Focus(target) => {
        Self::run_focus_command(target, &subject_container, state, config)
      }
      InvokeCommand::MoveWorkspace { direction } => {
        let workspace =
          subject_container.workspace().context("No workspace.")?;

        move_workspace_in_direction(&workspace, direction, state, config)
      }
      InvokeCommand::UpdateWorkspaceConfig {
        workspace,
        new_config,
      } => {
        let workspace = if let Some(workspace_name) = workspace {
          state
            .workspace_by_name(workspace_name)
            .context("Workspace doesn't exist.")?
        } else {
          subject_container.workspace().context("No workspace.")?
        };
        update_workspace_config(&workspace, state, config, new_config)
      }
      InvokeCommand::ShellExec {
        hide_window,
        command,
      } => shell_exec(&command.join(" "), *hide_window, state),
      InvokeCommand::ToggleTilingDirection => {
        toggle_tiling_direction(subject_container, state, config)
      }
      InvokeCommand::SetTilingDirection { tiling_direction } => {
        set_tiling_direction(
          subject_container,
          state,
          config,
          tiling_direction,
        )
      }
      InvokeCommand::WmCycleFocus { .. }
      | InvokeCommand::WmDisableBindingMode { .. }
      | InvokeCommand::WmEnableBindingMode { .. }
      | InvokeCommand::WmExit
      | InvokeCommand::WmRedraw
      | InvokeCommand::WmReloadConfig
      | InvokeCommand::WmTogglePause => {
        Self::run_wm_command(command, state, config)
      }
      // Commands that act on a window, and do nothing when the subject
      // container is not one.
      InvokeCommand::AdjustBorders(_)
      | InvokeCommand::Close
      | InvokeCommand::Ignore
      | InvokeCommand::Move(_)
      | InvokeCommand::Position(_)
      | InvokeCommand::Resize(_)
      | InvokeCommand::SetFloating { .. }
      | InvokeCommand::SetFullscreen { .. }
      | InvokeCommand::SetMinimized
      | InvokeCommand::SetTiling
      | InvokeCommand::SetTitleBarVisibility { .. }
      | InvokeCommand::SetTransparency(_)
      | InvokeCommand::Size(_)
      | InvokeCommand::ToggleFloating { .. }
      | InvokeCommand::ToggleFullscreen { .. }
      | InvokeCommand::ToggleMinimized
      | InvokeCommand::ToggleTiling => {
        match subject_container.as_window_container() {
          Ok(window) => {
            Self::run_window_command(command, window, state, config)
          }
          Err(_) => Ok(()),
        }
      }
    }
  }

  /// Runs a `focus` command.
  fn run_focus_command(
    target: &InvokeFocusCommand,
    subject_container: &Container,
    state: &mut WmState,
    config: &UserConfig,
  ) -> anyhow::Result<()> {
    match target {
      InvokeFocusCommand::Direction(direction) => {
        focus_in_direction(subject_container, direction, state)
      }
      InvokeFocusCommand::NextWindow => {
        focus_window_in_cycle(
          subject_container,
          FocusCycleDirection::Next,
          state,
        );
        Ok(())
      }
      InvokeFocusCommand::PrevWindow => {
        focus_window_in_cycle(
          subject_container,
          FocusCycleDirection::Previous,
          state,
        );
        Ok(())
      }
      InvokeFocusCommand::WorkspaceInDirection(direction) => {
        focus_workspace(
          WorkspaceTarget::Direction(direction.clone()),
          state,
          config,
        )
      }
      InvokeFocusCommand::ContainerId(container_id) => {
        focus_container_by_id(container_id, state)
      }
      InvokeFocusCommand::Workspace(name) => {
        focus_workspace(WorkspaceTarget::Name(name.clone()), state, config)
      }
      InvokeFocusCommand::Monitor(monitor_index) => {
        focus_monitor(*monitor_index, state, config)
      }
      InvokeFocusCommand::RelativeWorkspace(target) => {
        focus_workspace(WorkspaceTarget::from(*target), state, config)
      }
    }
  }

  /// Runs a command that acts on the WM itself rather than on the subject
  /// container.
  fn run_wm_command(
    command: &InvokeCommand,
    state: &mut WmState,
    config: &mut UserConfig,
  ) -> anyhow::Result<()> {
    match command {
      InvokeCommand::WmCycleFocus {
        omit_floating,
        omit_fullscreen,
        omit_minimized,
        omit_tiling,
      } => cycle_focus(
        |next| match next {
          WindowState::Floating(_) => *omit_floating,
          WindowState::Fullscreen(_) => *omit_fullscreen,
          WindowState::Minimized => *omit_minimized,
          WindowState::Tiling => *omit_tiling,
        },
        state,
        config,
      ),
      InvokeCommand::WmDisableBindingMode { name } => {
        disable_binding_mode(name, state);
        Ok(())
      }
      InvokeCommand::WmEnableBindingMode { name } => {
        enable_binding_mode(name, state, config)
      }
      InvokeCommand::WmExit => state.emit_exit(),
      InvokeCommand::WmRedraw => {
        state
          .pending_sync
          .queue_container_to_redraw(state.root_container.clone());

        Ok(())
      }
      InvokeCommand::WmReloadConfig => reload_config(state, config),
      InvokeCommand::WmTogglePause => {
        toggle_pause(state);
        Ok(())
      }
      // Every other command is handled by `run_command`.
      _ => Ok(()),
    }
  }

  /// Runs a `move` command.
  fn run_move_command(
    target: &InvokeMoveCommand,
    window: WindowContainer,
    state: &mut WmState,
    config: &UserConfig,
  ) -> anyhow::Result<()> {
    let workspace_target = match target {
      InvokeMoveCommand::Direction(direction) => {
        return move_window_in_direction(window, direction, state, config);
      }
      InvokeMoveCommand::WorkspaceInDirection(direction) => {
        WorkspaceTarget::Direction(direction.clone())
      }
      InvokeMoveCommand::Workspace(name) => {
        WorkspaceTarget::Name(name.clone())
      }
      InvokeMoveCommand::RelativeWorkspace(target) => {
        WorkspaceTarget::from(*target)
      }
    };

    move_window_to_workspace(window, workspace_target, state, config)
  }

  /// Runs a command that acts on a window.
  fn run_window_command(
    command: &InvokeCommand,
    window: WindowContainer,
    state: &mut WmState,
    config: &mut UserConfig,
  ) -> anyhow::Result<()> {
    match command {
      InvokeCommand::AdjustBorders(args) => {
        let args = args.clone();
        let border_delta = RectDelta::new(
          args.left.unwrap_or(LengthValue::from_px(0)),
          args.top.unwrap_or(LengthValue::from_px(0)),
          args.right.unwrap_or(LengthValue::from_px(0)),
          args.bottom.unwrap_or(LengthValue::from_px(0)),
        );

        window.set_border_delta(border_delta);
        state.pending_sync.queue_container_to_redraw(window);

        Ok(())
      }
      InvokeCommand::Close => {
        // Window handle might no longer be valid here.
        if let Err(err) = window.native().close() {
          warn!("Failed to close window: {:?}", err);
        }

        Ok(())
      }
      InvokeCommand::Ignore => ignore_window(&window, state),
      InvokeCommand::Move(target) => {
        Self::run_move_command(target, window, state, config)
      }
      InvokeCommand::Position(args) => {
        let target = if args.centered {
          WindowPositionTarget::Centered
        } else {
          WindowPositionTarget::Coordinates(args.x_pos, args.y_pos)
        };

        set_window_position(window, &target, state)
      }
      InvokeCommand::Resize(args) => resize_window(
        &window,
        args.width.clone(),
        args.height.clone(),
        state,
      ),
      InvokeCommand::Size(args) => set_window_size(
        window,
        args.width.clone(),
        args.height.clone(),
        state,
      ),
      #[cfg(target_os = "windows")]
      InvokeCommand::SetTitleBarVisibility { visibility } => {
        _ = window.native().set_title_bar_visibility(
          *visibility == TitleBarVisibility::Shown,
        );
        Ok(())
      }
      #[cfg(target_os = "windows")]
      InvokeCommand::SetTransparency(args) => {
        if let Some(opacity) = &args.opacity {
          _ = window.native().set_transparency(opacity);
        }

        if let Some(opacity_delta) = &args.opacity_delta {
          _ = window.native().adjust_transparency(opacity_delta);
        }

        Ok(())
      }
      InvokeCommand::SetFloating {
        centered,
        shown_on_top,
        x_pos,
        y_pos,
        width,
        height,
      } => Self::set_floating(
        window,
        FloatingPlacement {
          centered: *centered,
          shown_on_top: *shown_on_top,
          position: (*x_pos, *y_pos),
          size: (width.clone(), height.clone()),
        },
        state,
        config,
      ),
      _ => Self::run_window_state_command(command, window, state, config),
    }
  }

  /// Runs a command that changes a window's state, other than
  /// `set-floating`.
  fn run_window_state_command(
    command: &InvokeCommand,
    window: WindowContainer,
    state: &mut WmState,
    config: &mut UserConfig,
  ) -> anyhow::Result<()> {
    let state_defaults = &config.value.window_behavior.state_defaults;

    let target_state = match command {
      InvokeCommand::SetFullscreen {
        maximized,
        shown_on_top,
      } => WindowState::Fullscreen(FullscreenStateConfig {
        maximized: maximized
          .unwrap_or(state_defaults.fullscreen.maximized),
        shown_on_top: shown_on_top
          .unwrap_or(state_defaults.fullscreen.shown_on_top),
      }),
      InvokeCommand::SetMinimized => WindowState::Minimized,
      InvokeCommand::SetTiling => WindowState::Tiling,
      InvokeCommand::ToggleFloating {
        centered,
        shown_on_top,
      } => {
        let centered =
          centered.unwrap_or(state_defaults.floating.centered);
        let target_state = WindowState::Floating(FloatingStateConfig {
          centered,
          shown_on_top: shown_on_top
            .unwrap_or(state_defaults.floating.shown_on_top),
        });

        let window = update_window_state(
          window.clone(),
          window.toggled_state(target_state, config),
          state,
          config,
        )?;

        if !window.has_custom_floating_placement() && centered {
          set_window_position(
            window,
            &WindowPositionTarget::Centered,
            state,
          )?;
        }

        return Ok(());
      }
      InvokeCommand::ToggleFullscreen {
        maximized,
        shown_on_top,
      } => window.toggled_state(
        WindowState::Fullscreen(FullscreenStateConfig {
          maximized: maximized
            .unwrap_or(state_defaults.fullscreen.maximized),
          shown_on_top: shown_on_top
            .unwrap_or(state_defaults.fullscreen.shown_on_top),
        }),
        config,
      ),
      InvokeCommand::ToggleMinimized => {
        window.toggled_state(WindowState::Minimized, config)
      }
      InvokeCommand::ToggleTiling => {
        window.toggled_state(WindowState::Tiling, config)
      }
      // Every other command is handled by `run_command` or
      // `run_window_command` before reaching here.
      _ => return Ok(()),
    };

    update_window_state(window, target_state, state, config)?;
    Ok(())
  }

  /// Runs a `set-floating` command.
  fn set_floating(
    window: WindowContainer,
    placement: FloatingPlacement,
    state: &mut WmState,
    config: &mut UserConfig,
  ) -> anyhow::Result<()> {
    let floating_defaults =
      &config.value.window_behavior.state_defaults.floating;
    let centered =
      placement.centered.unwrap_or(floating_defaults.centered);

    let window = update_window_state(
      window,
      WindowState::Floating(FloatingStateConfig {
        centered,
        shown_on_top: placement
          .shown_on_top
          .unwrap_or(floating_defaults.shown_on_top),
      }),
      state,
      config,
    )?;

    // Allow size and position to be set if window has not previously
    // been manually placed.
    if !window.has_custom_floating_placement() {
      let (width, height) = placement.size;
      if width.is_some() || height.is_some() {
        set_window_size(window.clone(), width, height, state)?;
      }

      let (x_pos, y_pos) = placement.position;
      if centered {
        set_window_position(
          window,
          &WindowPositionTarget::Centered,
          state,
        )?;
      } else if x_pos.is_some() || y_pos.is_some() {
        set_window_position(
          window,
          &WindowPositionTarget::Coordinates(x_pos, y_pos),
          state,
        )?;
      }
    }

    Ok(())
  }

  /// Runs cleanup tasks when the WM is exiting.
  pub(crate) fn cleanup(
    &mut self,
    config: &mut UserConfig,
    ipc_server: &mut IpcServer,
  ) {
    self.state.emit_event(WmEvent::ApplicationExiting);

    // Ensure that the WM is unpaused, otherwise, shutdown commands won't
    // get executed.
    self.state.is_paused = false;

    // Run user's shutdown commands.
    if let Err(err) = self.process_commands(
      &config.value.general.shutdown_commands.clone(),
      None,
      config,
    ) {
      tracing::warn!("Failed to run shutdown commands: {:?}", err);
    }

    // Emit remaining WM events before exiting.
    while let Ok(wm_event) = self.event_rx.try_recv() {
      tracing::info!(
        "Emitting WM event before shutting down: {:?}",
        wm_event
      );

      if let Err(err) = ipc_server.process_event(wm_event) {
        tracing::warn!("{:?}", err);
      }
    }
  }
}
