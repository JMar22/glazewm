use crate::platform_impl;

/// Has the OS end this process only after ordinary applications when the
/// user signs out or shuts down.
///
/// The OS closes applications one at a time when the session ends, and an
/// application that blocks lets the user cancel. Cancelling spares only
/// the applications not yet reached: those closed before it stay closed.
/// A WM closed that way leaves the session unmanaged, and the windows it
/// hid on other workspaces invisible. Going last means a sign-out reaches
/// the WM only once every ordinary application has agreed to close.
///
/// The setting is per process. Processes this one starts begin at the
/// default level.
///
/// # Platform-specific
///
/// - **Windows**: Sets the process shutdown level to 0x100, the lowest
///   level reserved for applications. Processes start at 0x280, and
///   Windows ends higher levels first.
///
/// # Errors
///
/// Returns an error if the OS rejects the shutdown level.
pub fn shut_down_after_applications() -> crate::Result<()> {
  platform_impl::shut_down_after_applications()
}
