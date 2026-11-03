use std::cell::Cell;

use windows::Win32::{
  Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
  UI::{
    Input::KeyboardAndMouse::{
      GetKeyState, GetKeyboardLayout, VkKeyScanExW, VK_LCONTROL, VK_LMENU,
      VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
    },
    WindowsAndMessaging::{
      CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
      KBDLLHOOKSTRUCT, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
    },
  },
};

use crate::{Dispatcher, Key, KeyCode};

/// Callback stored in [`HOOK`] for intercepting keyboard events.
type HookCallback = Box<dyn Fn(KeyEvent) -> bool>;

thread_local! {
  /// Stores the hook callback for the current thread.
  ///
  /// The hook callback is called for every keyboard event and returns
  /// `true` if the event should be intercepted.
  static HOOK: Cell<Option<HookCallback>> = Cell::default();
}

/// A key event received from the keyboard hook.
#[derive(Clone, Debug)]
pub struct KeyEvent {
  /// The key that was pressed or released.
  pub key: Key,

  /// Whether the event is for a key press or release.
  pub is_keypress: bool,
}

impl KeyEvent {
  /// Gets whether the specified key is currently pressed.
  ///
  /// Reads the key state as of the keyboard event being processed, so
  /// call it from within the hook callback.
  pub fn is_key_down(key: Key) -> bool {
    match key {
      Key::Cmd | Key::Win => {
        Self::is_key_down_raw(VK_LWIN.0)
          || Self::is_key_down_raw(VK_RWIN.0)
      }
      Key::Alt => {
        Self::is_key_down_raw(VK_LMENU.0)
          || Self::is_key_down_raw(VK_RMENU.0)
      }
      Key::Ctrl => {
        Self::is_key_down_raw(VK_LCONTROL.0)
          || Self::is_key_down_raw(VK_RCONTROL.0)
      }
      Key::Shift => {
        Self::is_key_down_raw(VK_LSHIFT.0)
          || Self::is_key_down_raw(VK_RSHIFT.0)
      }
      _ => {
        if let Ok(key_code) = KeyCode::try_from(key) {
          Self::is_key_down_raw(key_code.0)
        } else {
          false
        }
      }
    }
  }

  /// Gets whether the specified key is currently down using the raw key
  /// code.
  fn is_key_down_raw(key: u16) -> bool {
    unsafe { (GetKeyState(key.into()) & 0x80) == 0x80 }
  }
}

/// Looks up the key that types `utf16_key` on the current keyboard
/// layout, with `VkKeyScanExW`.
///
/// The low-order byte of the result is the virtual-key code and the
/// high-order byte the shift state it needs, or the result is `-1` if no
/// key types the character.
pub(crate) fn scan_key_for_char(utf16_key: u16) -> i16 {
  let layout = unsafe { GetKeyboardLayout(0) };
  unsafe { VkKeyScanExW(utf16_key, layout) }
}

/// A system-wide low-level keyboard hook.
#[derive(Debug)]
pub struct KeyboardHook {
  handle: HHOOK,
  dispatcher: Dispatcher,
}

impl KeyboardHook {
  /// Creates an instance of `KeyboardHook`.
  ///
  /// The callback is called for every keyboard event and returns `true` if
  /// the event should be intercepted.
  ///
  /// # Panics
  ///
  /// Panics when attempting to register multiple hooks on the dispatcher's
  /// thread.
  pub fn new<F>(
    callback: F,
    dispatcher: &Dispatcher,
  ) -> crate::Result<Self>
  where
    F: Fn(KeyEvent) -> bool + Send + Sync + 'static,
  {
    let handle = dispatcher.dispatch_sync(move || {
      HOOK.with(|state| {
        assert!(
          state.take().is_none(),
          "Only one keyboard hook can be registered on the dispatcher's thread."
        );

        state.set(Some(Box::new(callback)));
      });

      unsafe {
        SetWindowsHookExW(
          WH_KEYBOARD_LL,
          Some(Self::hook_proc),
          HINSTANCE::default(),
          0,
        )
      }
    })??;

    Ok(Self {
      handle,
      dispatcher: dispatcher.clone(),
    })
  }

  /// Terminates the keyboard hook by unregistering it.
  pub fn terminate(&mut self) -> crate::Result<()> {
    unsafe { UnhookWindowsHookEx(self.handle) }?;

    // Dispatch cleanup to the event loop thread since the callback
    // is stored in a thread-local on that thread.
    let _ = self.dispatcher.dispatch_async(|| {
      HOOK.with(|state| {
        state.take();
      });
    });

    Ok(())
  }

  /// Hook procedure for keyboard events.
  ///
  /// For use with `SetWindowsHookExW`.
  extern "system" fn hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
  ) -> LRESULT {
    // If the code is less than zero, the hook procedure must pass the hook
    // notification directly to other applications.
    if code != 0 {
      return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // Get struct with the keyboard input event.
    let input = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };

    let is_keypress =
      matches!(u32::try_from(wparam.0), Ok(WM_KEYDOWN | WM_SYSKEYDOWN));

    // Virtual-key codes range from 1 to 254, so any key code that does not
    // fit is not a key to act on.
    let Some(key) = u16::try_from(input.vkCode)
      .ok()
      .and_then(|vk_code| Key::try_from(KeyCode(vk_code)).ok())
    else {
      return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    };

    let key_event = KeyEvent { key, is_keypress };

    let should_intercept = HOOK.with(|state| {
      if let Some(callback) = state.take() {
        let result = callback(key_event);
        state.set(Some(callback));
        result
      } else {
        false
      }
    });

    if should_intercept {
      return LRESULT(1);
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
  }
}

impl Drop for KeyboardHook {
  fn drop(&mut self) {
    let _ = self.terminate();
  }
}
