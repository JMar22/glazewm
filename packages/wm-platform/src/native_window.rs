#[cfg(target_os = "windows")]
use windows::Win32::{
  Foundation::HWND,
  UI::WindowsAndMessaging::{
    SET_WINDOW_POS_FLAGS, WINDOW_EX_STYLE, WINDOW_STYLE,
  },
};

use crate::{platform_impl, Rect};
#[cfg(target_os = "windows")]
use crate::{Color, CornerStyle, Delta, OpacityValue, RectDelta};

/// Unique identifier of a window.
///
/// Can be obtained with `window.id()`.
///
/// # Platform-specific
///
/// - **Windows**: `isize` (`HWND`)
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub isize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowZOrder {
  Normal,
  AfterWindow(WindowId),
  Top,
  TopMost,
}

/// Windows-specific extensions for [`NativeWindow`].
#[cfg(target_os = "windows")]
pub trait NativeWindowWindowsExt {
  /// Creates a [`NativeWindow`] from a window handle.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn from_handle(handle: isize) -> NativeWindow;

  /// Gets the window handle.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn hwnd(&self) -> HWND;

  /// Gets the class name of the window.
  ///
  /// # Errors
  ///
  /// Returns an error if `GetClassNameW` fails, e.g. because the window
  /// no longer exists.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn class_name(&self) -> crate::Result<String>;

  /// Gets the window's frame, including the window's shadow borders.
  ///
  /// # Errors
  ///
  /// Returns an error if `GetWindowRect` fails, e.g. because the window
  /// no longer exists.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn frame_with_shadows(&self) -> crate::Result<Rect>;

  /// Gets the delta between the window's frame and the window's border.
  /// This represents the size of a window's shadow borders.
  ///
  /// # Errors
  ///
  /// Returns an error if the window's frame or its frame with shadows
  /// can't be read (see [`NativeWindow::frame`] and
  /// [`NativeWindowWindowsExt::frame_with_shadows`]).
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn shadow_borders(&self) -> crate::Result<RectDelta>;

  /// Whether the window has an owner window.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn has_owner_window(&self) -> bool;

  /// The window's owner, if it has one.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn owner_window(&self) -> Option<NativeWindow>;

  /// Whether the window has the given window style flag(s) set.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn has_window_style(&self, style: WINDOW_STYLE) -> bool;

  /// Whether the window has the given extended window style flag(s) set.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn has_window_style_ex(&self, style: WINDOW_EX_STYLE) -> bool;

  /// Thin wrapper around [`SetWindowPos`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos).
  ///
  /// # Errors
  ///
  /// Returns an error if `SetWindowPos` fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_window_pos(
    &self,
    z_order: &WindowZOrder,
    rect: &Rect,
    flags: SET_WINDOW_POS_FLAGS,
  ) -> crate::Result<()>;

  /// Shows the window asynchronously.
  ///
  /// NOTE: Cloaked windows do not get shown until uncloaked.
  ///
  /// # Errors
  ///
  /// Returns an error if `ShowWindowAsync` fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn show(&self) -> crate::Result<()>;

  /// Hides the window asynchronously.
  ///
  /// # Errors
  ///
  /// Returns an error if `ShowWindowAsync` fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn hide(&self) -> crate::Result<()>;

  /// Restores the window (unminimizes and unmaximizes).
  ///
  /// If `outer_frame` is provided, the window will be restored to the
  /// specified position. This avoids flickering compared to restoring
  /// and then repositioning the window.
  ///
  /// # Errors
  ///
  /// Returns an error if `ShowWindowAsync` fails or, when `outer_frame` is
  /// provided, if `SetWindowPlacement` fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn restore(&self, outer_frame: Option<&Rect>) -> crate::Result<()>;

  /// Cloaks or uncloaks the window.
  ///
  /// # Errors
  ///
  /// Returns an error if the shell's COM interfaces are unavailable, if no
  /// application view exists for the window, or if cloaking it fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_cloaked(&self, cloaked: bool) -> crate::Result<()>;

  /// Whether the window has the `WS_VISIBLE` style, whether or not it is
  /// cloaked.
  ///
  /// Cloaking leaves `WS_VISIBLE` in place, so with the cloak hide method
  /// a window without it was hidden by its application (e.g. `SW_HIDE`
  /// when it closes to the tray), not by the WM.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn has_visible_style(&self) -> bool;

  /// Marks the window as fullscreen.
  ///
  /// Causes the native Windows taskbar to be moved to the bottom of the
  /// z-order when this window is active.
  ///
  /// # Errors
  ///
  /// Returns an error if the taskbar's COM interface is unavailable or
  /// `MarkFullscreenWindow` fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn mark_fullscreen(&self, fullscreen: bool) -> crate::Result<()>;

  /// Gets the window directly above this one in the z-order.
  ///
  /// Returns `None` once the top of the stack is reached, so repeated
  /// calls walk everything drawn over this window.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn window_above(&self) -> Option<crate::NativeWindow>;

  /// Adds or removes the window from the native taskbar.
  ///
  /// Cloaked windows are normally always shown in the taskbar, but can be
  /// manually toggled. Hidden windows (`SW_HIDE`) can never be shown in
  /// the taskbar.
  ///
  /// # Errors
  ///
  /// Returns an error if the taskbar's COM interface is unavailable or
  /// adding or removing the window's tab fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_taskbar_visibility(&self, visible: bool) -> crate::Result<()>;

  /// Adds the given extended window style flag(s) to the window.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn add_window_style_ex(&self, style: WINDOW_EX_STYLE);

  /// Sets the window's z-order.
  ///
  /// # Errors
  ///
  /// Returns an error if `SetWindowPos` fails. The follow-up call made
  /// shortly afterwards, in case the z-order did not stick, is not
  /// checked.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_z_order(&self, zorder: &WindowZOrder) -> crate::Result<()>;

  /// Sets the visibility of the window's title bar.
  ///
  /// # Errors
  ///
  /// Returns an error if the window's style changes and the `SetWindowPos`
  /// call that applies the change fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_title_bar_visibility(&self, visible: bool) -> crate::Result<()>;

  /// Sets the color of the window's border.
  ///
  /// # Errors
  ///
  /// Returns an error if `DwmSetWindowAttribute` fails, e.g. on Windows
  /// versions without the border color attribute.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_border_color(&self, color: Option<&Color>) -> crate::Result<()>;

  /// Sets the corner style of the window.
  ///
  /// # Errors
  ///
  /// Returns an error if `DwmSetWindowAttribute` fails, e.g. on Windows
  /// versions without the corner preference attribute.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_corner_style(
    &self,
    corner_style: &CornerStyle,
  ) -> crate::Result<()>;

  /// Sets the transparency of the window.
  ///
  /// # Errors
  ///
  /// Returns an error if `SetLayeredWindowAttributes` fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn set_transparency(
    &self,
    opacity_value: &OpacityValue,
  ) -> crate::Result<()>;

  /// Adjusts the window's transparency by a relative delta.
  ///
  /// # Errors
  ///
  /// Returns an error if the window's current transparency can't be read,
  /// if the window uses a color key rather than alpha for its transparency
  /// ([`Error::Platform`](crate::Error::Platform)), or if setting the new
  /// transparency fails.
  ///
  /// # Platform-specific
  ///
  /// This method is only available on Windows.
  fn adjust_transparency(
    &self,
    opacity_delta: &Delta<OpacityValue>,
  ) -> crate::Result<()>;
}

#[cfg(target_os = "windows")]
impl NativeWindowWindowsExt for NativeWindow {
  fn from_handle(handle: isize) -> Self {
    platform_impl::NativeWindow::new(handle).into()
  }

  fn hwnd(&self) -> HWND {
    self.inner.hwnd()
  }

  fn class_name(&self) -> crate::Result<String> {
    self.inner.class_name()
  }

  fn frame_with_shadows(&self) -> crate::Result<Rect> {
    self.inner.frame_with_shadows()
  }

  fn shadow_borders(&self) -> crate::Result<RectDelta> {
    self.inner.shadow_borders()
  }

  fn has_owner_window(&self) -> bool {
    self.inner.has_owner_window()
  }

  fn owner_window(&self) -> Option<NativeWindow> {
    self.inner.owner_window()
  }

  fn has_window_style(&self, style: WINDOW_STYLE) -> bool {
    self.inner.has_window_style(style)
  }

  fn has_window_style_ex(&self, style: WINDOW_EX_STYLE) -> bool {
    self.inner.has_window_style_ex(style)
  }

  fn set_window_pos(
    &self,
    z_order: &WindowZOrder,
    rect: &Rect,
    flags: SET_WINDOW_POS_FLAGS,
  ) -> crate::Result<()> {
    self.inner.set_window_pos(z_order, rect, flags)
  }

  fn show(&self) -> crate::Result<()> {
    self.inner.show()
  }

  fn hide(&self) -> crate::Result<()> {
    self.inner.hide()
  }

  fn restore(&self, outer_frame: Option<&Rect>) -> crate::Result<()> {
    self.inner.restore(outer_frame)
  }

  fn set_cloaked(&self, cloaked: bool) -> crate::Result<()> {
    self.inner.set_cloaked(cloaked)
  }

  fn has_visible_style(&self) -> bool {
    self.inner.has_visible_style()
  }

  fn mark_fullscreen(&self, fullscreen: bool) -> crate::Result<()> {
    self.inner.mark_fullscreen(fullscreen)
  }

  fn window_above(&self) -> Option<crate::NativeWindow> {
    self.inner.window_above()
  }

  fn set_taskbar_visibility(&self, visible: bool) -> crate::Result<()> {
    self.inner.set_taskbar_visibility(visible)
  }

  fn add_window_style_ex(&self, style: WINDOW_EX_STYLE) {
    self.inner.add_window_style_ex(style);
  }

  fn set_z_order(&self, z_order: &WindowZOrder) -> crate::Result<()> {
    self.inner.set_z_order(z_order)
  }

  fn set_title_bar_visibility(&self, visible: bool) -> crate::Result<()> {
    self.inner.set_title_bar_visibility(visible)
  }

  fn set_border_color(&self, color: Option<&Color>) -> crate::Result<()> {
    self.inner.set_border_color(color)
  }

  fn set_corner_style(
    &self,
    corner_style: &CornerStyle,
  ) -> crate::Result<()> {
    self.inner.set_corner_style(corner_style)
  }

  fn set_transparency(
    &self,
    opacity_value: &OpacityValue,
  ) -> crate::Result<()> {
    self.inner.set_transparency(opacity_value)
  }

  fn adjust_transparency(
    &self,
    opacity_delta: &Delta<OpacityValue>,
  ) -> crate::Result<()> {
    self.inner.adjust_transparency(opacity_delta)
  }
}

#[derive(Clone, Debug)]
pub struct NativeWindow {
  pub(crate) inner: platform_impl::NativeWindow,
}

impl NativeWindow {
  /// Gets the unique identifier for this window.
  #[must_use]
  pub fn id(&self) -> WindowId {
    self.inner.id()
  }

  /// Gets the window's title.
  ///
  /// Note that empty strings are valid window titles.
  ///
  /// # Platform-specific
  ///
  /// - **Windows**: A window that no longer exists reads as having an
  ///   empty title.
  #[must_use]
  pub fn title(&self) -> String {
    self.inner.title()
  }

  /// Gets the file name of the window's process executable, without its
  /// extension.
  ///
  /// # Errors
  ///
  /// Returns an error if the window's process can't be opened or queried
  /// for its executable path.
  pub fn process_name(&self) -> crate::Result<String> {
    self.inner.process_name()
  }

  /// Gets a rectangle of the window's size and position.
  ///
  /// # Platform-specific
  ///
  /// - **Windows**: Includes the window's shadow borders.
  ///
  /// # Errors
  ///
  /// Returns an error if the frame can't be read from DWM or from
  /// `GetWindowRect`, e.g. because the window no longer exists.
  pub fn frame(&self) -> crate::Result<Rect> {
    self.inner.frame()
  }

  /// Gets the window's position as (x, y) coordinates.
  ///
  /// # Errors
  ///
  /// Returns an error if the window's frame can't be read (see
  /// [`NativeWindow::frame`]).
  pub fn position(&self) -> crate::Result<(f64, f64)> {
    self.inner.position()
  }

  /// Gets the window's size as (width, height).
  ///
  /// # Errors
  ///
  /// Returns an error if the window's frame can't be read (see
  /// [`NativeWindow::frame`]).
  pub fn size(&self) -> crate::Result<(f64, f64)> {
    self.inner.size()
  }

  /// Whether the window is still valid.
  ///
  /// Returns `true` if the underlying window is still alive.
  #[must_use]
  pub fn is_valid(&self) -> bool {
    self.inner.is_valid()
  }

  /// Whether the window is actually visible.
  ///
  /// # Errors
  ///
  /// Returns an error if the window's cloaked state can't be read from
  /// DWM.
  pub fn is_visible(&self) -> crate::Result<bool> {
    self.inner.is_visible()
  }

  /// Whether the window is minimized.
  #[must_use]
  pub fn is_minimized(&self) -> bool {
    self.inner.is_minimized()
  }

  /// Whether the window is maximized.
  #[must_use]
  pub fn is_maximized(&self) -> bool {
    self.inner.is_maximized()
  }

  /// Whether the window can be resized.
  #[must_use]
  pub fn is_resizable(&self) -> bool {
    self.inner.is_resizable()
  }

  /// Whether the window is the OS's desktop window.
  #[must_use]
  pub fn is_desktop_window(&self) -> bool {
    self.inner.is_desktop_window()
  }

  /// Repositions and resizes the window to the specified rectangle.
  ///
  /// # Platform-specific
  ///
  /// - **Windows**: Automatically adjusts the `rect` prior to calling [`SetWindowPos`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos)
  ///   to include the window's shadow borders. To set the window's
  ///   position directly, use [`NativeWindowWindowsExt::set_window_pos`].
  ///
  /// # Errors
  ///
  /// Returns an error if `SetWindowPos` fails.
  pub fn set_frame(&self, rect: &Rect) -> crate::Result<()> {
    self.inner.set_frame(rect)
  }

  /// Resizes the window to the specified size.
  ///
  /// # Errors
  ///
  /// Returns an error if `SetWindowPos` fails.
  pub fn resize(&self, width: i32, height: i32) -> crate::Result<()> {
    self.inner.resize(width, height)
  }

  /// Repositions the window to the specified position.
  ///
  /// # Errors
  ///
  /// Returns an error if `SetWindowPos` fails.
  pub fn reposition(&self, x: i32, y: i32) -> crate::Result<()> {
    self.inner.reposition(x, y)
  }

  /// Minimizes the window asynchronously.
  ///
  /// # Errors
  ///
  /// Returns an error if `ShowWindowAsync` fails.
  pub fn minimize(&self) -> crate::Result<()> {
    self.inner.minimize()
  }

  /// Maximizes the window asynchronously.
  ///
  /// # Errors
  ///
  /// Returns an error if `ShowWindowAsync` fails.
  pub fn maximize(&self) -> crate::Result<()> {
    self.inner.maximize()
  }

  /// Sets focus to the window and raises it to the top of the z-order.
  ///
  /// # Errors
  ///
  /// Returns [`Error::Platform`](crate::Error::Platform) if Windows refuses
  /// to make the window the foreground window, which it does while another
  /// process holds the foreground.
  pub fn focus(&self) -> crate::Result<()> {
    self.inner.focus()
  }

  /// Closes the window.
  ///
  /// # Platform-specific
  ///
  /// - **Windows**: This sends a `WM_CLOSE` message to the window.
  ///
  /// # Errors
  ///
  /// Returns an error if sending the message fails.
  pub fn close(&self) -> crate::Result<()> {
    self.inner.close()
  }
}

impl PartialEq for NativeWindow {
  fn eq(&self, other: &Self) -> bool {
    self.inner.id() == other.inner.id()
  }
}

impl Eq for NativeWindow {}
