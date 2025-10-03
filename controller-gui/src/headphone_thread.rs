use std::time::Duration;

use bluer::{
    Device, Session, Uuid,
    rfcomm::{Profile, Role},
};
use eframe::egui::Context;
use futures::StreamExt;
use log::debug;
use sony_wf1000xm5::{
    MessageType,
    command::Command,
    frame_parser::{FrameParser, FrameParserResult},
    message::Payload,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
const SONY_SERVICE_UUID: Uuid = Uuid::from_u128(0x956C7B26_D49A_4BA8_B03F_B17D393CB6E2);

#[tokio::main(flavor = "current_thread")]
pub async fn thread_main(
    device: Device,
    payload_tx: mpsc::UnboundedSender<Payload>,
    mut command_rx: mpsc::UnboundedReceiver<Command>,
    mut stop_rx: mpsc::Receiver<()>,
    ctx: Context,
) -> bluer::Result<()> {
    debug!("attempting to connect...");
    device.connect().await?;
    debug!("connected!");
    let profile = Profile {
        uuid: SONY_SERVICE_UUID,
        role: Some(Role::Client),
        auto_connect: Some(true),
        ..Default::default()
    };
    let session = Session::new().await?;
    let mut profile_handle = session.register_profile(profile).await?;
    let connection = tokio::select! {
        Some(connection_request) = profile_handle.next() => {
            connection_request
        }

        _ = tokio::time::sleep(Duration::from_secs(5)) => {
            debug!("(exiting with an error)");
            return Err(bluer::Error { kind: bluer::ErrorKind::DoesNotExist, message: "Unable to connect to sony service. Are you sure it's a WF-1000XM5?".to_string() })
        }
    };
    debug!("connection request: {:?}", connection);
    let mut stream = connection.accept()?;
    debug!("connection accepted!");
    let mut buffer = [0];
    let mut frame_parser = FrameParser::new();
    let mut seq_number = 0;
    let init_command = sony_wf1000xm5::command::build_command(&Command::Init, seq_number);
    debug!("init_command: {:x?}", init_command);
    let mut tries = 3;
    stream.write_all(&init_command).await.unwrap();
    loop {
        tokio::select! {
            Ok(_) = stream.peek(&mut buffer) => {
                break;
            }

            _ =  tokio::time::sleep(Duration::from_millis(1500)) => {
                if tries == 0 {
                    // random errorkind but who cares
                    return Err(bluer::Error { kind: bluer::ErrorKind::AuthenticationTimeout, message: "max retries failed; try connecting again".to_string() })
                }
                debug!("init failed; retrying...");
                stream.write_all(&init_command).await.unwrap();
                tries -= 1;
            }


        }
    }

    // communication must be done sequentially, so after a command we must wait for an Ack
    let mut waiting_for_ack = false;
    'eventloop: loop {
        tokio::select! {

            _ = stop_rx.recv() => {
                return Ok(());
            }
            Ok(_) = stream.peek(&mut buffer) => {

            while stream.read(&mut buffer).await.is_ok() {
                match frame_parser.parse(&buffer) {
                    FrameParserResult::Ready { buf, .. } => {
                        let msg = match sony_wf1000xm5::message::parse_message(buf)  {
                            Ok(m) => m,
                            Err(e) => {
                                log::warn!("error while parsing message: {e}");
                                continue;
                            }
                        };
                        debug!("msg: {:x?}", msg);
                        if msg.kind == MessageType::Ack {
                            seq_number = msg.seq_num;
                            waiting_for_ack = false;
                            break;
                        } else if msg.kind == MessageType::Command1 {
                            let payload = sony_wf1000xm5::message::parse_payload(msg.payload);
                            debug!("payload: {:x?}", payload);

                            let command = sony_wf1000xm5::command::build_command(&Command::Ack, msg.seq_num);
                            debug!("responding: {:x?}", command);
                            stream.write_all(&command).await?;

                            match payload {
                                Ok(payload) => {
                                    if payload_tx.send(payload).is_err() {
                                        break 'eventloop;
                                    }
                                    ctx.request_repaint();
                                }

                                Err(e) => {
                                    log::warn!("bad payload: {e}");
                                }

                            }
                            // we sent Ack, we're done with this message
                            break;
                        }
                    }
                    FrameParserResult::Incomplete { .. } => {
                        // we read another byte
                    }

                    FrameParserResult::Error { err, consumed } => {
                        log::warn!("frame parser returned an error: {err}, consumed: {consumed}");
                        return Err(bluer::Error { kind: bluer::ErrorKind::AuthenticationTimeout, message: "FrameParser failed. It is likely that the headphone sent a malformed request. Reconnect.".to_string() })
                    }
                }
            }
        }

            Some(command) = command_rx.recv(), if !waiting_for_ack => {
                let command = sony_wf1000xm5::command::build_command(&command, seq_number);
                debug!("sending: {:?}", command);
                stream
                .write_all(&command)
                .await?;
                waiting_for_ack = true;
            }
        }
    }

    Ok(())
}
