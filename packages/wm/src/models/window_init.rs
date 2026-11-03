use uuid::Uuid;
use wm_common::{ActiveDrag, WindowRuleConfig, WindowState};
use wm_platform::{NativeWindow, Rect, RectDelta};

use crate::models::NativeWindowProperties;

/// What a window is created with, whether tiling or not.
///
/// A window converted between [`TilingWindow`](super::TilingWindow) and
/// [`NonTilingWindow`](super::NonTilingWindow) carries all of this over.
pub struct WindowInit {
  /// ID to keep across a conversion, or `None` to assign a new one.
  pub id: Option<Uuid>,
  pub native: NativeWindow,
  pub properties: NativeWindowProperties,
  pub prev_state: Option<WindowState>,
  pub border_delta: RectDelta,
  pub floating_placement: Rect,
  pub has_custom_floating_placement: bool,
  pub done_window_rules: Vec<WindowRuleConfig>,
  pub active_drag: Option<ActiveDrag>,
}
