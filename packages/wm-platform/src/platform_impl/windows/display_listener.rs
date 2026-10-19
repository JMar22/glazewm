use std::sync::{
  atomic::{AtomicBool, AtomicIsize, Ordering},
  Arc,
};

use tokio::sync::mpsc;
use tracing::warn;
use windows::{
  core::GUID,
  Win32::{
    Foundation::HANDLE,
    System::{
      Power::{
        RegisterPowerSettingNotification,
        UnregisterPowerSettingNotification, HPOWERNOTIFY,
        POWERBROADCAST_SETTING,
      },
      SystemServices::GUID_CONSOLE_DISPLAY_STATE,
    },
    UI::WindowsAndMessaging::{
      DBT_DEVNODES_CHANGED, DEVICE_NOTIFY_WINDOW_HANDLE,
      PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
      PBT_POWERSETTINGCHANGE, SPI_SETWORKAREA, WM_DEVICECHANGE,
      WM_DISPLAYCHANGE, WM_POWERBROADCAST, WM_SETTINGCHANGE,
    },
  },
};

use crate::{Dispatcher, DispatcherExtWindows};

/// Whether the console display is currently off.
///
/// Kept current by the power-setting notification that
/// [`DisplayListener`] registers. Reads as on until the first
/// notification arrives, so a missing notification leaves the display
/// treated as on rather than silencing the callers that check it.
static IS_DISPLAY_OFF: AtomicBool = AtomicBool::new(false);

/// Gets whether the console display is currently off.
pub(crate) fn is_display_off() -> bool {
  IS_DISPLAY_OFF.load(Ordering::Relaxed)
}

/// Reads whether the display is off from the payload of a
/// `GUID_CONSOLE_DISPLAY_STATE` power-setting notification.
///
/// The payload is a `DWORD`: 0 is off, 1 is on and 2 is dimmed. Returns
/// `None` for any other setting or a payload of an unexpected size.
fn console_display_is_off(
  setting: &GUID,
  data_length: u32,
  state: u32,
) -> Option<bool> {
  (*setting == GUID_CONSOLE_DISPLAY_STATE
    && data_length as usize == std::mem::size_of::<u32>())
  .then_some(state == 0)
}

/// Reads the console display state out of the `POWERBROADCAST_SETTING`
/// that accompanies a `PBT_POWERSETTINGCHANGE`.
fn console_display_is_off_from_message(lparam: isize) -> Option<bool> {
  let setting = lparam as *const POWERBROADCAST_SETTING;
  if setting.is_null() {
    return None;
  }

  // SAFETY: For `PBT_POWERSETTINGCHANGE`, `lParam` points to a
  // `POWERBROADCAST_SETTING` that stays valid while the message is being
  // handled. Its `Data` member is only the first byte of a payload
  // `DataLength` bytes long, so the payload is read from its address
  // rather than through the one-byte array, and only once the length has
  // been checked to cover a `DWORD`.
  unsafe {
    let data_length = (*setting).DataLength;
    if data_length as usize != std::mem::size_of::<u32>() {
      return None;
    }

    let state = std::ptr::addr_of!((*setting).Data)
      .cast::<u32>()
      .read_unaligned();

    console_display_is_off(&(*setting).PowerSetting, data_length, state)
  }
}

/// Platform-specific implementation of [`DisplayListener`].
pub(crate) struct DisplayListener {
  callback_id: Option<usize>,
  dispatcher: Dispatcher,

  /// Handle of the console display-state notification, or 0 until it has
  /// been registered on the event loop thread.
  power_notification: Arc<AtomicIsize>,
}

impl DisplayListener {
  /// Implements [`DisplayListener::new`].
  pub(crate) fn new(
    event_tx: mpsc::UnboundedSender<()>,
    dispatcher: &Dispatcher,
  ) -> crate::Result<Self> {
    let is_system_suspended = Arc::new(AtomicBool::new(false));

    let callback_id = dispatcher.register_wndproc_callback(Box::new(
      move |_hwnd, message, wparam, lparam| {
        match message {
          WM_POWERBROADCAST => {
            #[allow(clippy::cast_possible_truncation)]
            match wparam as u32 {
              // The console display turned on, off or dimmed.
              PBT_POWERSETTINGCHANGE => {
                if let Some(is_off) =
                  console_display_is_off_from_message(lparam)
                {
                  IS_DISPLAY_OFF.store(is_off, Ordering::Relaxed);
                }
              }
              // System is resuming from sleep/hibernation.
              PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                is_system_suspended.store(false, Ordering::Relaxed);
              }
              // System is entering sleep/hibernation.
              PBT_APMSUSPEND => {
                is_system_suspended.store(true, Ordering::Relaxed);
              }
              _ => {}
            }

            Some(0)
          }
          WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DEVICECHANGE => {
            let should_emit = {
              // Ignore display change messages if the system hasn't fully
              // resumed from sleep.
              if is_system_suspended.load(Ordering::Relaxed) {
                false
              } else {
                #[allow(clippy::cast_possible_truncation)]
                match message {
                  // Received when displays are connected and disconnected,
                  // resolution changes, or arrangement changes.
                  WM_DISPLAYCHANGE => true,
                  // Received when the working area has changed. Fires when
                  // the Windows taskbar is changed or an appbar is
                  // registered or changed. 3rd-party apps like
                  // ButteryTaskbar can trigger this message by calling
                  // `SystemParametersInfo(SPI_SETWORKAREA, ...)`.
                  WM_SETTINGCHANGE => wparam as u32 == SPI_SETWORKAREA.0,
                  // Received when any device is connected or disconnected
                  // (including non-display devices).
                  // TODO: Check if this is actually needed. Previous C#
                  // implementation did not use this.
                  WM_DEVICECHANGE => wparam as u32 == DBT_DEVNODES_CHANGED,
                  _ => unreachable!(),
                }
              }
            };

            if should_emit {
              let _ = event_tx.send(());
            }

            Some(0)
          }
          _ => None,
        }
      },
    ))?;

    // Queued after the callback above, which is installed through the same
    // queue, so the notification Windows sends on registering for the
    // current state is not delivered before there is a callback to take it.
    let power_notification = Arc::new(AtomicIsize::new(0));
    let message_window = dispatcher.message_window_handle();
    let registered = power_notification.clone();
    dispatcher.dispatch_async(move || {
      let result = unsafe {
        RegisterPowerSettingNotification(
          HANDLE(message_window),
          std::ptr::from_ref(&GUID_CONSOLE_DISPLAY_STATE),
          DEVICE_NOTIFY_WINDOW_HANDLE.0,
        )
      };

      match result {
        Ok(handle) => registered.store(handle.0, Ordering::Relaxed),
        Err(err) => {
          warn!("Failed to watch the display's power state: {err}");
        }
      }
    })?;

    Ok(Self {
      callback_id: Some(callback_id),
      dispatcher: dispatcher.clone(),
      power_notification,
    })
  }

  /// Implements [`DisplayListener::terminate`].
  pub(crate) fn terminate(&mut self) -> crate::Result<()> {
    if let Some(id) = self.callback_id.take() {
      self.dispatcher.deregister_wndproc_callback(id)?;
    }

    // Unregistering is queued behind the registration, so a listener that
    // is dropped straight after it is created still releases the handle.
    let power_notification = self.power_notification.clone();
    self.dispatcher.dispatch_async(move || {
      let handle = power_notification.swap(0, Ordering::Relaxed);
      if handle != 0 {
        let _ = unsafe {
          UnregisterPowerSettingNotification(HPOWERNOTIFY(handle))
        };
      }

      IS_DISPLAY_OFF.store(false, Ordering::Relaxed);
    })?;

    Ok(())
  }
}

impl Drop for DisplayListener {
  fn drop(&mut self) {
    if let Err(err) = self.terminate() {
      warn!("Failed to terminate display listener: {}", err);
    }
  }
}

#[cfg(test)]
mod tests {
  use windows::{
    core::GUID,
    Win32::System::SystemServices::{
      GUID_CONSOLE_DISPLAY_STATE, GUID_SESSION_DISPLAY_STATUS,
    },
  };

  use super::console_display_is_off;

  const DWORD_SIZE: u32 = 4;

  fn display_is_off(
    setting: &GUID,
    data_length: u32,
    state: u32,
  ) -> Option<bool> {
    console_display_is_off(setting, data_length, state)
  }

  #[test]
  fn reads_the_off_state_and_nothing_else_as_off() {
    // The payload is 0 for off, 1 for on and 2 for dimmed. A dimmed
    // display is still showing, so only 0 means the user has lost the
    // screen.
    assert_eq!(
      display_is_off(&GUID_CONSOLE_DISPLAY_STATE, DWORD_SIZE, 0),
      Some(true)
    );
    assert_eq!(
      display_is_off(&GUID_CONSOLE_DISPLAY_STATE, DWORD_SIZE, 1),
      Some(false)
    );
    assert_eq!(
      display_is_off(&GUID_CONSOLE_DISPLAY_STATE, DWORD_SIZE, 2),
      Some(false)
    );
  }

  #[test]
  fn ignores_other_settings_and_malformed_payloads() {
    // The same window receives every power setting it registers for, so
    // another setting's zero must not read as the display going off.
    assert_eq!(
      display_is_off(&GUID_SESSION_DISPLAY_STATUS, DWORD_SIZE, 0),
      None
    );

    // A payload that is not a `DWORD` cannot be trusted to be one.
    assert_eq!(display_is_off(&GUID_CONSOLE_DISPLAY_STATE, 1, 0), None);
    assert_eq!(display_is_off(&GUID_CONSOLE_DISPLAY_STATE, 8, 0), None);
  }
}
