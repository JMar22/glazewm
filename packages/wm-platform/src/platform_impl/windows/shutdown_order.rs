use windows::Win32::System::Threading::SetProcessShutdownParameters;

/// Lowest shutdown level reserved for applications. The levels below it
/// are reserved for system components.
const LAST_APPLICATION_SHUTDOWN_LEVEL: u32 = 0x100;

/// Implements [`crate::shut_down_after_applications`].
pub(crate) fn shut_down_after_applications() -> crate::Result<()> {
  unsafe {
    SetProcessShutdownParameters(LAST_APPLICATION_SHUTDOWN_LEVEL, 0)
  }?;

  Ok(())
}
