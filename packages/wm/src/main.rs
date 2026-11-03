// The `windows` or `console` subsystem (default is `console`) determines
// whether a console window is spawned on launch, if not already ran
// through a console. The following prevents this additional console window
// in release mode.
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
#![warn(clippy::all, clippy::pedantic)]

use std::{env, path::PathBuf, process, time::Duration};

use anyhow::{Context, Error};
use tokio::{process::Command, signal};
use tracing::Level;
use tracing_subscriber::{
  fmt::{self, writer::MakeWriterExt},
  layer::SubscriberExt,
};
use wm_common::{
  AppCommand, BindingModeConfig, InvokeCommand, Verbosity, WmEvent,
};
use wm_platform::{
  Dispatcher, DisplayListener, EventLoop, Keybinding, KeybindingListener,
  MouseEventKind, MouseListener, PlatformEvent, SingleInstance,
  WindowListener,
};

use crate::{
  commands::general::{
    reconcile_stranded_focus, reconcile_stranded_taskbar,
  },
  ipc_server::IpcServer,
  sys_tray::SystemTray,
  user_config::UserConfig,
  wm::WindowManager,
};

mod commands;
mod events;
mod ipc_server;
mod models;
mod pending_sync;
mod sys_tray;
mod traits;
mod user_config;
mod wm;
mod wm_state;

#[cfg(test)]
mod test_utils;

/// Main entry point for the application.
///
/// Conditionally starts the WM or runs a CLI command based on the given
/// subcommand.
fn main() -> anyhow::Result<()> {
  let args = std::env::args().collect::<Vec<_>>();
  let app_command = AppCommand::parse_with_default(&args);

  if let AppCommand::Start {
    config_path,
    verbosity,
  } = app_command
  {
    let rt = tokio::runtime::Runtime::new()?;
    let (event_loop, dispatcher) = EventLoop::new()?;

    let task_handle = std::thread::spawn(move || {
      rt.block_on(async {
        let start_res =
          start_wm(config_path, verbosity, &dispatcher).await;

        if let Err(err) = &start_res {
          // If unable to start the WM, the error is fatal and a message
          // dialog is shown.
          tracing::error!("{:?}", err);
          dispatcher.show_error_dialog("Fatal error", &err.to_string());
        }

        if let Err(err) = dispatcher.stop_event_loop() {
          // Forcefully exit the process to ensure the event loop is
          // stopped.
          tracing::error!("Failed to stop event loop gracefully: {}", err);
          process::exit(1);
        }

        start_res
      })
    });

    // Run event loop (blocks until shutdown).
    event_loop.run()?;

    // Wait for clean exit of the WM.
    task_handle.join().unwrap()
  } else {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(wm_cli::start(args))
  }
}

async fn start_wm(
  config_path: Option<PathBuf>,
  verbosity: Verbosity,
  dispatcher: &Dispatcher,
) -> anyhow::Result<()> {
  setup_logging(&verbosity)?;

  // Ensure that only one instance of the WM is running.
  let _single_instance = SingleInstance::new()?;

  // Parse and validate user config.
  let mut config = UserConfig::new(config_path)?;

  // Add application icon to system tray.
  let mut tray = SystemTray::new(&config.path, dispatcher.clone())?;

  let mut wm = WindowManager::new(&mut config, dispatcher.clone())?;

  let mut ipc_server = IpcServer::start().await?;

  // Start watcher process for restoring hidden windows on crash.
  #[cfg(target_os = "windows")]
  start_watcher();

  // Start listening for platform events after populating initial state.
  let mut listeners = Listeners::new(&config, dispatcher)?;

  run_startup_commands(&mut wm, &mut config, dispatcher);

  run_event_loop(
    &mut wm,
    &mut config,
    &mut tray,
    &mut ipc_server,
    &mut listeners,
    dispatcher,
  )
  .await?;

  tracing::info!("Window manager shutting down.");
  wm.cleanup(&mut config, &mut ipc_server);

  Ok(())
}

/// Platform event listeners that feed the event loop.
struct Listeners {
  window: WindowListener,
  display: DisplayListener,
  mouse: MouseListener,
  keybinding: KeybindingListener,
}

impl Listeners {
  fn new(
    config: &UserConfig,
    dispatcher: &Dispatcher,
  ) -> anyhow::Result<Self> {
    Ok(Self {
      window: WindowListener::new(dispatcher)?,
      display: DisplayListener::new(dispatcher)?,
      mouse: MouseListener::new(mouse_events(config), dispatcher)?,
      keybinding: KeybindingListener::new(
        &active_keybindings(config, &[]),
        dispatcher,
      )?,
    })
  }
}

/// Handles platform events, timers, IPC messages and WM events until the
/// WM is told to exit.
///
/// Errors from handling an event are reported without stopping the loop.
/// The loop itself stops with an error only if the mouse listener can't be
/// updated for a config change.
async fn run_event_loop(
  wm: &mut WindowManager,
  config: &mut UserConfig,
  tray: &mut SystemTray,
  ipc_server: &mut IpcServer,
  listeners: &mut Listeners,
  dispatcher: &Dispatcher,
) -> anyhow::Result<()> {
  let mut timers = Timers::new();

  loop {
    let res = tokio::select! {
      _ = signal::ctrl_c() => {
        tracing::info!("Received SIGINT signal.");
        break;
      },
      Some(()) = wm.exit_rx.recv() => {
        tracing::info!("Exiting through WM command.");
        break;
      },
      Some(()) = tray.exit_rx.recv() => {
        tracing::info!("Exiting through system tray.");
        break;
      },
      Some(event) = listeners.mouse.next_event() => {
        tracing::debug!("Received mouse event: {:?}", event);
        wm.process_event(PlatformEvent::Mouse(event), config)
      },
      Some(event) = listeners.window.next_event() => {
        tracing::debug!("Received window event: {:?}", event);
        wm.process_event(PlatformEvent::Window(event), config)
      },
      Some(()) = listeners.display.next_event() => {
        tracing::debug!("Received display settings changed event.");
        wm.process_event(PlatformEvent::DisplaySettingsChanged, config)
      },
      Some(event) = listeners.keybinding.next_event() => {
        tracing::debug!("Received keyboard event: {:?}", event);
        wm.process_event(PlatformEvent::Keybinding(event), config)
      }
      _ = timers.focus_reconcile.tick() => {
        reconcile_stranded_focus(&mut wm.state, config);
        Ok(())
      },
      _ = timers.taskbar_reconcile.tick() => {
        reconcile_stranded_taskbar(&wm.state);
        Ok(())
      },
      _ = timers.cleanup.tick() => {
        if wm.state.is_paused {
          Ok(())
        } else {
          wm.state.cleanup_invalid_windows()
        }
      },
      Some((
        message,
        response_tx,
        disconnection_tx
      )) = ipc_server.message_rx.recv() => {
        tracing::info!("Received IPC message: {:?}", message);

        if let Err(err) = ipc_server.process_message(
          message,
          &response_tx,
          &disconnection_tx,
          wm,
          config,
        ) {
          tracing::error!("{:?}", err);
        }

        Ok(())
      },
      Some(wm_event) = wm.event_rx.recv() => {
        tracing::debug!("Received WM event: {:?}", wm_event);

        update_listeners(
          &wm_event,
          config,
          &wm.state.binding_modes,
          &mut listeners.mouse,
          &listeners.keybinding,
        )?;

        if let Err(err) = ipc_server.process_event(wm_event) {
          tracing::error!("{:?}", err);
        }

        Ok(())
      },
      Some(()) = tray.config_reload_rx.recv() => {
        wm.process_commands(
          &vec![InvokeCommand::WmReloadConfig],
          None,
          config,
        ).map(|_| ())
      },
    };

    if let Err(err) = res {
      tracing::error!("{:?}", err);
      dispatcher.show_error_dialog("Non-fatal error", &err.to_string());
    }
  }

  Ok(())
}

/// Runs the user's startup commands, reporting a failure without stopping
/// the WM.
fn run_startup_commands(
  wm: &mut WindowManager,
  config: &mut UserConfig,
  dispatcher: &Dispatcher,
) {
  if let Err(err) = wm.process_commands(
    &config.value.general.startup_commands.clone(),
    None,
    config,
  ) {
    tracing::error!("{:?}", err);
    dispatcher.show_error_dialog("Non-fatal error", &err.to_string());
  }
}

/// Timers for the event loop's periodic checks.
struct Timers {
  /// Cleans up invalid windows.
  cleanup: tokio::time::Interval,

  /// Recovers keyboard focus stranded on the Windows shell. Dismissing a
  /// taskbar flyout can leave focus nowhere without emitting any
  /// foreground event, so that state has to be polled for. Kept short,
  /// since the desktop is unusable until it resolves.
  focus_reconcile: tokio::time::Interval,

  /// Puts the taskbar back behind a fullscreen window after something
  /// raises it. Kept short because the taskbar is drawn over the window
  /// until it fires, and the check costs nothing while no window is
  /// covering the taskbar, which is nearly all of the time.
  taskbar_reconcile: tokio::time::Interval,
}

impl Timers {
  fn new() -> Self {
    Self {
      cleanup: interval(Duration::from_secs(5)),
      focus_reconcile: interval(Duration::from_secs(1)),
      taskbar_reconcile: interval(Duration::from_millis(500)),
    }
  }
}

/// Creates a timer for a periodic task. Ticks missed while the event loop
/// is busy are skipped rather than run back to back.
fn interval(period: Duration) -> tokio::time::Interval {
  let mut interval = tokio::time::interval(period);
  interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  interval
}

/// Mouse events to listen for under the given config.
fn mouse_events(config: &UserConfig) -> &'static [MouseEventKind] {
  if config.value.general.focus_follows_cursor {
    &[MouseEventKind::Move, MouseEventKind::LeftButtonUp]
  } else {
    &[MouseEventKind::LeftButtonUp]
  }
}

/// Keybindings to listen for with the given binding modes active.
fn active_keybindings(
  config: &UserConfig,
  binding_modes: &[BindingModeConfig],
) -> Vec<Keybinding> {
  config
    .active_keybinding_configs(binding_modes, false)
    .flat_map(|kb| kb.bindings)
    .collect()
}

/// Updates the mouse and keybinding listeners for a WM event that changes
/// what they should listen for.
fn update_listeners(
  wm_event: &WmEvent,
  config: &UserConfig,
  binding_modes: &[BindingModeConfig],
  mouse_listener: &mut MouseListener,
  keybinding_listener: &KeybindingListener,
) -> anyhow::Result<()> {
  // Disable mouse listener when the WM is paused.
  if let WmEvent::PauseChanged { is_paused } = wm_event {
    let _ = mouse_listener.enable(!is_paused);
  }

  // Update keybinding and mouse listeners on config changes.
  if matches!(
    wm_event,
    WmEvent::UserConfigChanged { .. }
      | WmEvent::BindingModesChanged { .. }
      | WmEvent::PauseChanged { .. }
  ) {
    keybinding_listener.update(&active_keybindings(config, binding_modes));
    mouse_listener.set_enabled_events(mouse_events(config))?;
  }

  Ok(())
}

/// Starts the watcher process that restores hidden windows if the WM
/// crashes. A failure to start it is logged rather than returned.
#[cfg(target_os = "windows")]
fn start_watcher() {
  if let Err(err) = start_watcher_process() {
    tracing::warn!(
      "Failed to start watcher process: {err}{}",
      cfg!(debug_assertions)
        .then_some(".\n Run `cargo build -p wm-watcher` to build it.")
        .unwrap_or_default()
    );
  }
}

/// Initialize logging with the specified verbosity level.
///
/// Error logs are saved to `~/.glzr/glazewm/errors.log`.
fn setup_logging(verbosity: &Verbosity) -> anyhow::Result<()> {
  let error_log_dir = home::home_dir()
    .context("Unable to get home directory.")?
    .join(".glzr/glazewm/");

  let error_writer =
    tracing_appender::rolling::never(error_log_dir, "errors.log");

  let subscriber = tracing_subscriber::registry()
    .with(
      // Output to stdout with specified verbosity level.
      fmt::Layer::new()
        .with_writer(std::io::stdout.with_max_level(verbosity.level())),
    )
    .with(
      // Output to error log file.
      fmt::Layer::new()
        .with_writer(error_writer.with_max_level(Level::ERROR)),
    );

  tracing::subscriber::set_global_default(subscriber)?;

  tracing::info!(
    "Starting WM with log level {:?}.",
    verbosity.level().to_string()
  );

  Ok(())
}

/// Launches watcher binary (Windows-only). This is a separate process that
/// is responsible for restoring hidden windows in case the main WM process
/// crashes.
///
/// This assumes the watcher binary exists in the same directory as the
/// WM binary.
fn start_watcher_process() -> anyhow::Result<tokio::process::Child, Error>
{
  let watcher_path = env::current_exe()?
    .parent()
    .context("Failed to resolve path to the watcher process.")?
    .join("glazewm-watcher");

  Command::new(&watcher_path)
    .spawn()
    .context("Failed to start watcher process.")
}
