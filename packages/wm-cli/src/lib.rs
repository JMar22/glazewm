#![warn(clippy::all, clippy::pedantic)]

use anyhow::Context;
use wm_common::ClientResponseData;
use wm_ipc_client::IpcClient;

/// Sends the command in `args` (after the program name) to the running WM
/// and prints its response as JSON.
///
/// Event subscriptions keep printing each event as it arrives.
///
/// # Errors
///
/// Returns an error if the WM's IPC server can't be reached, if sending
/// the command or receiving a response fails, or if a response can't be
/// serialized for printing.
pub async fn start(args: Vec<String>) -> anyhow::Result<()> {
  let mut client = IpcClient::connect().await?;

  let message = args[1..].join(" ");
  client
    .send(&message)
    .await
    .context("Failed to send command to IPC server.")?;

  let client_response = client
    .client_response(&message)
    .await
    .context("Failed to receive response from IPC server.")?;

  match client_response.data {
    // For event subscriptions, omit the initial response message and
    // continuously output subsequent event messages.
    Some(ClientResponseData::EventSubscribe(data)) => loop {
      let event_subscription = client
        .event_subscription(&data.subscription_id)
        .await
        .context("Failed to receive response from IPC server.")?;

      println!("{}", serde_json::to_string(&event_subscription)?);
    },
    // For all other messages, output and exit when the first response
    // message is received.
    _ => {
      println!("{}", serde_json::to_string(&client_response)?);
    }
  }

  Ok(())
}
