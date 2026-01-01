#[cfg(not(target_arch = "wasm32"))]
use bluer::{
    Device, Session, Uuid,
    rfcomm::{Profile, Role},
};
use eframe::egui::Context;
#[cfg(not(target_arch = "wasm32"))]
use futures::StreamExt;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, pin_mut};

use anyhow::bail;
use log::debug;
use sony_wf1000xm5::{
    MessageType,
    command::Command,
    frame_parser::{FrameParser, FrameParserResult},
    payload::Payload,
};
#[cfg(target_arch = "wasm32")]
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::mpsc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::JsFuture;
#[cfg(target_arch = "wasm32")]
use wasm_streams::{
    ReadableStream, WritableStream, readable::IntoAsyncRead, writable::IntoAsyncWrite,
};
#[cfg(target_arch = "wasm32")]
use web_sys::SerialPort;
#[cfg(not(target_arch = "wasm32"))]
const SONY_SERVICE_UUID: Uuid = Uuid::from_u128(0x956C7B26_D49A_4BA8_B03F_B17D393CB6E2);

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main(flavor = "current_thread")]
pub async fn thread_main(
    device: Device,
    payload_tx: mpsc::UnboundedSender<Payload>,
    command_rx: mpsc::UnboundedReceiver<Command>,
    mut stop_rx: mpsc::Receiver<()>,
    ctx: Context,
) -> anyhow::Result<()> {
    use tokio_util::compat::TokioAsyncReadCompatExt;

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
        _ = stop_rx.recv() => {
            return Ok(());
        }
        Some(connection_request) = profile_handle.next() => {
            connection_request
        }

        _ = tokio::time::sleep(Duration::from_secs(5)) => {
            debug!("(exiting with an error)");
            bail!("Unable to connect to sony service. Are you sure it's a WF-1000XM5?");
        }
    };
    debug!("connection request: {:?}", connection);
    let stream = connection.accept()?;
    let stream = stream.compat();
    connect(stream, payload_tx, command_rx, stop_rx, ctx).await?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub async fn thread_main(
    port: SerialPort,
    payload_tx: mpsc::UnboundedSender<Payload>,
    command_rx: mpsc::UnboundedReceiver<Command>,
    stop_rx: mpsc::Receiver<()>,
    ctx: Context,
) -> anyhow::Result<()> {
    use web_sys::SerialOptions;

    if let Err(e) = JsFuture::from(port.open(&SerialOptions::new(9600))).await {
        bail!("Couldn't open serial port: {e:?}");
    };
    let writeable_stream = WritableStream::from_raw(port.writable()).into_async_write();
    let readable_stream = ReadableStream::from_raw(port.readable()).into_async_read();
    let web_stream = WebSerialStream {
        readable_stream,
        writeable_stream,
    };
    let ctxx = ctx.clone();
    connect(web_stream, payload_tx, command_rx, stop_rx, ctx).await?;
    if let Err(e) = JsFuture::from(port.close()).await {
        bail!("Couldn't close serial port: {e:?}");
    };
    debug!("thread main died peacefully");
    // notify the GUI about it
    ctxx.request_repaint();
    Ok(())
}

// could've just lived with 2 separate streams instead of combining them into 1 struct which implements AsyncRead and AsyncWrite... but it's already done so
#[cfg(target_arch = "wasm32")]
struct WebSerialStream {
    readable_stream: IntoAsyncRead<'static>,
    writeable_stream: IntoAsyncWrite<'static>,
}

#[cfg(target_arch = "wasm32")]
impl AsyncRead for WebSerialStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.readable_stream).poll_read(cx, buf)
    }
}

#[cfg(target_arch = "wasm32")]
impl AsyncWrite for WebSerialStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.writeable_stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.writeable_stream).poll_flush(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.writeable_stream).poll_close(cx)
    }
}

async fn connect(
    stream: impl AsyncRead + AsyncWrite,
    payload_tx: mpsc::UnboundedSender<Payload>,
    mut command_rx: mpsc::UnboundedReceiver<Command>,
    mut stop_rx: mpsc::Receiver<()>,
    ctx: Context,
) -> anyhow::Result<()> {
    let mut frame_parser = FrameParser::new();
    let mut seq_number = 0;
    let init_command = sony_wf1000xm5::command::build_command(&Command::Init, seq_number);
    debug!("init_command: {:x?}", init_command);
    let mut tries = 3;
    pin_mut!(stream);
    stream.write_all(&init_command).await?;
    let mut buffer = [0];
    let sleep = async |duration| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            tokio::time::sleep(duration).await
        }
        #[cfg(target_arch = "wasm32")]
        {
            gloo_timers::future::sleep(duration).await
        }
    };

    loop {
        tokio::select! {
            _ = stop_rx.recv() => {
                return Ok(());
            }

            Ok(_) = stream.read(&mut buffer) => {
                // stream is alive
                break;
            }

            _ =  sleep(Duration::from_millis(1500)) => {
                if tries == 0 {
                    anyhow::bail!("max retries failed; try connecting again");
                }
                debug!("init failed; retrying...");
                stream.write_all(&init_command).await?;
                tries -= 1;
            }


        }
    }
    // feed the 1 byte we read
    frame_parser.parse(&buffer);

    // communication must be done sequentially, so after a command we must wait for an Ack
    // (we start with true because we wait for Ack for our init)
    let mut waiting_for_ack = true;
    'eventloop: loop {
        tokio::select! {

            _ = stop_rx.recv() => {
                debug!("event loop received stop");
                return Ok(());
            }
            Ok(n) = stream.read(&mut buffer) => {
                let mut offset = 0;
                loop {
                    match frame_parser.parse(&buffer[offset..n]) {

                        FrameParserResult::Ready { msg, consumed} => {
                            if let Err(e) = msg.kind {
                                log::warn!("unknown message type: {e}; ignoring");
                                continue;
                            }
                            if let Err(e) = msg.checksum.as_ref() {
                                log::warn!("bad checksum: {e}; ignoring");
                                continue;
                            }
                            debug!("msg: {msg:x?}");
                            if msg.kind == Ok(MessageType::Ack) {
                                seq_number = msg.seq_num;
                                waiting_for_ack = false;
                            } else if msg.kind == Ok(MessageType::Command1) || msg.kind == Ok(MessageType::Command2) {
                                let payload = sony_wf1000xm5::payload::parse_payload(msg.payload, msg.kind.unwrap());
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
                            }

                            offset += consumed;
                            if offset >=  n {
                                // we're done reading
                                break;
                            }
                        }

                        FrameParserResult::Incomplete { .. } => {
                            // we read more bytes
                            break;
                        }

                        FrameParserResult::Error { err, consumed } => {
                            log::warn!("frame parser returned an error: {err}, consumed: {consumed}");
                            anyhow::bail!("FrameParser failed. It is likely that the headphone sent a malformed request. Reconnect.");
                        }


                    }
                }

        }

            Some(command) = command_rx.recv(), if !waiting_for_ack => {
                let command_bytes = sony_wf1000xm5::command::build_command(&command, seq_number);
                debug!("sending: {:?}, raw: {:x?}", command, command_bytes);
                stream
                .write_all(&command_bytes)
                .await?;
                waiting_for_ack = true;
            }
        }
    }

    Ok(())
}
