//! Arguments of the `focus` and `move` commands.
//!
//! Each command takes exactly one target, so both are enums. Clap can't
//! derive `Args` for an enum, so the arguments are built by hand here. They
//! match what deriving `Args` on a struct of optional fields and flags
//! produced, including the flags, help text and error messages, and the
//! serialized form of each command keeps the same JSON shape.

use clap::{
  builder::ValueParser, error::ErrorKind, value_parser, Arg, ArgAction,
  ArgGroup, ArgMatches, Args, Command, FromArgMatches,
};
use serde::{ser::SerializeStruct, Serialize, Serializer};
use uuid::Uuid;
use wm_platform::Direction;

/// A workspace relative to the current one, picked with a plain flag such
/// as `--next-workspace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeWorkspace {
  NextActive,
  PrevActive,
  Next,
  Prev,
  NextActiveOnMonitor,
  PrevActiveOnMonitor,
  Recent,
}

impl RelativeWorkspace {
  /// Each target with its argument ID and flag, in help order.
  const FLAGS: [(Self, &'static str, &'static str); 7] = [
    (
      Self::NextActive,
      "next_active_workspace",
      "next-active-workspace",
    ),
    (
      Self::PrevActive,
      "prev_active_workspace",
      "prev-active-workspace",
    ),
    (Self::Next, "next_workspace", "next-workspace"),
    (Self::Prev, "prev_workspace", "prev-workspace"),
    (
      Self::NextActiveOnMonitor,
      "next_active_workspace_on_monitor",
      "next-active-workspace-on-monitor",
    ),
    (
      Self::PrevActiveOnMonitor,
      "prev_active_workspace_on_monitor",
      "prev-active-workspace-on-monitor",
    ),
    (Self::Recent, "recent_workspace", "recent-workspace"),
  ];

  fn add_args(command: Command) -> Command {
    Self::FLAGS.iter().fold(command, |command, &(_, id, long)| {
      command.arg(flag(id, long, None))
    })
  }

  fn from_matches(matches: &ArgMatches) -> Option<Self> {
    Self::FLAGS
      .iter()
      .find(|&&(_, id, _)| matches.get_flag(id))
      .map(|&(target, ..)| target)
  }

  /// Serializes each flag as a field that is `true` for `target`.
  fn serialize_fields<S: SerializeStruct>(
    state: &mut S,
    target: Option<Self>,
  ) -> Result<(), S::Error> {
    for (flag_target, id, _) in Self::FLAGS {
      state.serialize_field(id, &(target == Some(flag_target)))?;
    }

    Ok(())
  }
}

/// Target of the `focus` command.
#[derive(Clone, Debug, PartialEq)]
pub enum InvokeFocusCommand {
  Direction(Direction),

  /// Focus the next window with the same state in the current workspace.
  NextWindow,

  /// Focus the previous window with the same state in the current
  /// workspace.
  PrevWindow,

  ContainerId(Uuid),
  WorkspaceInDirection(Direction),
  Workspace(String),
  Monitor(usize),
  RelativeWorkspace(RelativeWorkspace),
}

impl InvokeFocusCommand {
  const GROUP: &'static str = "InvokeFocusCommand";
}

impl Args for InvokeFocusCommand {
  fn group_id() -> Option<clap::Id> {
    Some(clap::Id::from(Self::GROUP))
  }

  fn augment_args(command: Command) -> Command {
    let command = command
      .arg(option(
        ("direction", "direction", "DIRECTION"),
        value_parser!(Direction),
        None,
      ))
      .arg(flag(
        "next_window",
        "next-window",
        Some(
          "Focus the next window with the same state in the current \
           workspace",
        ),
      ))
      .arg(flag(
        "prev_window",
        "prev-window",
        Some(
          "Focus the previous window with the same state in the current \
           workspace",
        ),
      ))
      .arg(option(
        ("container_id", "container-id", "CONTAINER_ID"),
        value_parser!(Uuid),
        None,
      ))
      .arg(option(
        (
          "workspace_in_direction",
          "workspace-in-direction",
          "WORKSPACE_IN_DIRECTION",
        ),
        value_parser!(Direction),
        None,
      ))
      .arg(option(
        ("workspace", "workspace", "WORKSPACE"),
        value_parser!(String),
        None,
      ))
      .arg(option(
        ("monitor", "monitor", "MONITOR"),
        value_parser!(usize),
        None,
      ));

    RelativeWorkspace::add_args(command).group(one_of(
      Self::GROUP,
      &[
        "direction",
        "next_window",
        "prev_window",
        "container_id",
        "workspace_in_direction",
        "workspace",
        "monitor",
      ],
    ))
  }

  fn augment_args_for_update(command: Command) -> Command {
    Self::augment_args(command)
  }
}

impl FromArgMatches for InvokeFocusCommand {
  fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
    if let Some(direction) = matches.get_one::<Direction>("direction") {
      return Ok(Self::Direction(direction.clone()));
    }

    if matches.get_flag("next_window") {
      return Ok(Self::NextWindow);
    }

    if matches.get_flag("prev_window") {
      return Ok(Self::PrevWindow);
    }

    if let Some(id) = matches.get_one::<Uuid>("container_id") {
      return Ok(Self::ContainerId(*id));
    }

    if let Some(direction) =
      matches.get_one::<Direction>("workspace_in_direction")
    {
      return Ok(Self::WorkspaceInDirection(direction.clone()));
    }

    if let Some(name) = matches.get_one::<String>("workspace") {
      return Ok(Self::Workspace(name.clone()));
    }

    if let Some(index) = matches.get_one::<usize>("monitor") {
      return Ok(Self::Monitor(*index));
    }

    RelativeWorkspace::from_matches(matches)
      .map(Self::RelativeWorkspace)
      .ok_or_else(|| missing_target(Self::GROUP))
  }

  fn update_from_arg_matches(
    &mut self,
    matches: &ArgMatches,
  ) -> Result<(), clap::Error> {
    *self = Self::from_arg_matches(matches)?;
    Ok(())
  }
}

impl Serialize for InvokeFocusCommand {
  fn serialize<S: Serializer>(
    &self,
    serializer: S,
  ) -> Result<S::Ok, S::Error> {
    let mut state = serializer.serialize_struct(Self::GROUP, 14)?;

    state.serialize_field(
      "direction",
      &match self {
        Self::Direction(direction) => Some(direction),
        _ => None,
      },
    )?;
    state
      .serialize_field("next_window", &matches!(self, Self::NextWindow))?;
    state
      .serialize_field("prev_window", &matches!(self, Self::PrevWindow))?;
    state.serialize_field(
      "container_id",
      &match self {
        Self::ContainerId(id) => Some(id),
        _ => None,
      },
    )?;
    state.serialize_field(
      "workspace_in_direction",
      &match self {
        Self::WorkspaceInDirection(direction) => Some(direction),
        _ => None,
      },
    )?;
    state.serialize_field(
      "workspace",
      &match self {
        Self::Workspace(name) => Some(name),
        _ => None,
      },
    )?;
    state.serialize_field(
      "monitor",
      &match self {
        Self::Monitor(index) => Some(index),
        _ => None,
      },
    )?;
    RelativeWorkspace::serialize_fields(
      &mut state,
      match self {
        Self::RelativeWorkspace(target) => Some(*target),
        _ => None,
      },
    )?;

    state.end()
  }
}

/// Target of the `move` command.
#[derive(Clone, Debug, PartialEq)]
pub enum InvokeMoveCommand {
  /// Direction to move the window.
  Direction(Direction),

  /// Move window to workspace in specified direction.
  WorkspaceInDirection(Direction),

  /// Name of workspace to move the window.
  Workspace(String),

  RelativeWorkspace(RelativeWorkspace),
}

impl InvokeMoveCommand {
  const GROUP: &'static str = "InvokeMoveCommand";
}

impl Args for InvokeMoveCommand {
  fn group_id() -> Option<clap::Id> {
    Some(clap::Id::from(Self::GROUP))
  }

  fn augment_args(command: Command) -> Command {
    let command = command
      .arg(option(
        ("direction", "direction", "DIRECTION"),
        value_parser!(Direction),
        Some("Direction to move the window"),
      ))
      .arg(option(
        (
          "workspace_in_direction",
          "workspace-in-direction",
          "WORKSPACE_IN_DIRECTION",
        ),
        value_parser!(Direction),
        Some("Move window to workspace in specified direction"),
      ))
      .arg(option(
        ("workspace", "workspace", "WORKSPACE"),
        value_parser!(String),
        Some("Name of workspace to move the window"),
      ));

    RelativeWorkspace::add_args(command).group(one_of(
      Self::GROUP,
      &["direction", "workspace_in_direction", "workspace"],
    ))
  }

  fn augment_args_for_update(command: Command) -> Command {
    Self::augment_args(command)
  }
}

impl FromArgMatches for InvokeMoveCommand {
  fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
    if let Some(direction) = matches.get_one::<Direction>("direction") {
      return Ok(Self::Direction(direction.clone()));
    }

    if let Some(direction) =
      matches.get_one::<Direction>("workspace_in_direction")
    {
      return Ok(Self::WorkspaceInDirection(direction.clone()));
    }

    if let Some(name) = matches.get_one::<String>("workspace") {
      return Ok(Self::Workspace(name.clone()));
    }

    RelativeWorkspace::from_matches(matches)
      .map(Self::RelativeWorkspace)
      .ok_or_else(|| missing_target(Self::GROUP))
  }

  fn update_from_arg_matches(
    &mut self,
    matches: &ArgMatches,
  ) -> Result<(), clap::Error> {
    *self = Self::from_arg_matches(matches)?;
    Ok(())
  }
}

impl Serialize for InvokeMoveCommand {
  fn serialize<S: Serializer>(
    &self,
    serializer: S,
  ) -> Result<S::Ok, S::Error> {
    let mut state = serializer.serialize_struct(Self::GROUP, 10)?;

    state.serialize_field(
      "direction",
      &match self {
        Self::Direction(direction) => Some(direction),
        _ => None,
      },
    )?;
    state.serialize_field(
      "workspace_in_direction",
      &match self {
        Self::WorkspaceInDirection(direction) => Some(direction),
        _ => None,
      },
    )?;
    state.serialize_field(
      "workspace",
      &match self {
        Self::Workspace(name) => Some(name),
        _ => None,
      },
    )?;
    RelativeWorkspace::serialize_fields(
      &mut state,
      match self {
        Self::RelativeWorkspace(target) => Some(*target),
        _ => None,
      },
    )?;

    state.end()
  }
}

/// An optional `--long <VALUE>` argument, as derived for an `Option<T>`
/// field.
fn option(
  (id, long, value_name): (&'static str, &'static str, &'static str),
  parser: impl Into<ValueParser>,
  help: Option<&'static str>,
) -> Arg {
  Arg::new(id)
    .long(long)
    .value_name(value_name)
    .value_parser(parser)
    .action(ArgAction::Set)
    .help(help)
}

/// A `--long` flag, as derived for a `bool` field.
fn flag(
  id: &'static str,
  long: &'static str,
  help: Option<&'static str>,
) -> Arg {
  Arg::new(id)
    .long(long)
    .action(ArgAction::SetTrue)
    .help(help)
}

/// A group requiring exactly one of `ids` or a relative workspace flag.
fn one_of(group: &'static str, ids: &[&'static str]) -> ArgGroup {
  let flag_ids = RelativeWorkspace::FLAGS.iter().map(|&(_, id, _)| id);

  ArgGroup::new(group)
    .args(ids.iter().copied().chain(flag_ids))
    .required(true)
    .multiple(false)
}

/// Error for matches without a target, which the required group rules out
/// before parsing gets this far.
fn missing_target(group: &str) -> clap::Error {
  clap::Error::raw(
    ErrorKind::MissingRequiredArgument,
    format!("one of the `{group}` arguments is required"),
  )
}
