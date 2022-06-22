pub mod source_manager;

use std::time;

use tokio::select;
use tokio::time::timeout;
use tokio::sync::mpsc;
use tokio::sync::broadcast;

use oddity_video as video;

use crate::runtime::Runtime;
use crate::runtime::task_manager::{Task, TaskContext};
use crate::media::{self, MediaDescriptor};
use crate::media::video::reader::StreamReader;

pub enum SourceState {
  Stopped(SourcePath),
}

pub type SourceStateTx = mpsc::UnboundedSender<SourceState>;
pub type SourceStateRx = mpsc::UnboundedReceiver<SourceState>;

pub type SourceMediaInfoTx = broadcast::Sender<media::MediaInfo>;
pub type SourceMediaInfoRx = broadcast::Receiver<media::MediaInfo>;

pub type SourceStreamStateTx = broadcast::Sender<media::StreamState>;
pub type SourceStreamStateRx = broadcast::Receiver<media::StreamState>;

pub type SourceResetTx = broadcast::Sender<media::MediaInfo>;
pub type SourceResetRx = broadcast::Receiver<media::MediaInfo>;

pub type SourcePacketTx = broadcast::Sender<media::Packet>;
pub type SourcePacketRx = broadcast::Receiver<media::Packet>;

pub enum SourceControlMessage {
  StreamInfo,
  StreamState,
}

pub type SourceControlTx = mpsc::UnboundedSender<SourceControlMessage>;
pub type SourceControlRx = mpsc::UnboundedReceiver<SourceControlMessage>;

pub struct Source {
  pub name: String,
  pub path: SourcePath,
  pub descriptor: MediaDescriptor,
  control_tx: SourceControlTx,
  media_info_tx: SourceMediaInfoTx,
  stream_state_tx: SourceStreamStateTx,
  reset_tx: SourceResetTx,
  packet_tx: SourcePacketTx,
  worker: Task,
}

impl Source {
  /// Any more than 16 media/stream info messages on the queue probably means
  /// something is really wrong and the server is overloaded.
  const MAX_QUEUED_INFO: usize = 16;

  /// Any more than 1024 packets queued probably indicates the server is
  /// terribly overloaded/broken.
  const MAX_QUEUED_PACKETS: usize = 1024;

  /// Number of seconds between retries.
  const RETRY_DELAY_SECS: u64 = 60;

  pub async fn start(
    name: &str,
    path: SourcePath,
    descriptor: MediaDescriptor,
    state_tx: SourceStateTx,
    runtime: &Runtime,
  ) -> Result<Self, video::Error> {
    let path = normalize_path(path);
    let stream_reader = StreamReader::new(&descriptor).await?;

    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (media_info_tx, _) = broadcast::channel(Self::MAX_QUEUED_INFO);
    let (stream_state_tx, _) = broadcast::channel(Self::MAX_QUEUED_INFO);
    let (reset_tx, _) = broadcast::channel(Self::MAX_QUEUED_INFO);
    let (packet_tx, _) = broadcast::channel(Self::MAX_QUEUED_PACKETS);

    tracing::trace!(name, %path, "starting source");
    let worker = runtime
      .task()
      .spawn({
        let path = path.clone();
        let descriptor = descriptor.clone();
        let media_info_tx = media_info_tx.clone();
        let stream_state_tx = stream_state_tx.clone();
        let reset_tx = reset_tx.clone();
        let packet_tx = packet_tx.clone();
        move |task_context| {
          Self::run(
            path,
            descriptor,
            stream_reader,
            control_rx,
            state_tx,
            media_info_tx,
            stream_state_tx,
            reset_tx,
            packet_tx,
            task_context,
          )
        }
      })
      .await;
    tracing::trace!(name, %path, "started source");

    Ok(Self {
      name: name.to_string(),
      path,
      descriptor,
      control_tx,
      media_info_tx,
      stream_state_tx,
      reset_tx,
      packet_tx,
      worker,
    })
  }

  pub async fn stop(&mut self) {
    tracing::trace!("sending stop signal to source");
    self.worker.stop().await;
    tracing::trace!("stopped source");
  }

  pub fn delegate(&mut self) -> SourceDelegate {
    SourceDelegate {
      control_tx: self.control_tx.clone(),
      media_info_rx: self.media_info_tx.subscribe(),
      stream_state_rx: self.stream_state_tx.subscribe(),
      reset_rx: self.reset_tx.subscribe(),
      packet_rx: self.packet_tx.subscribe(),
    }
  }

  async fn run(
    path: SourcePath,
    descriptor: MediaDescriptor,
    mut stream_reader: StreamReader,
    mut control_rx: SourceControlRx,
    state_tx: SourceStateTx,
    media_info_tx: SourceMediaInfoTx,
    stream_state_tx: SourceStreamStateTx,
    reset_tx: SourceResetTx,
    packet_tx: SourcePacketTx,
    mut task_context: TaskContext,
  ) {
    'outer: loop {
      'inner: loop {
        select! {
          // CANCEL SAFETY: `StreamReader::read` uses `mpsc::UnboundedReceiver::recv`
          // internally which is cancel safe.
          packet = stream_reader.read() => {
            match packet {
              Some(Ok(packet)) => {
                let _ = packet_tx.send(packet.clone());
              },
              Some(Err(err)) => {
                tracing::error!(%path, %err, "failed to read video stream");

                // We can assume more failures are coming from the reader so let's
                // break out of the reading loop and restart it.
                break 'inner;
              },
              None => {
                tracing::error!(%path, "stream reader broken unexpectedly");
                break 'inner;
              },
            };
          },
          // CANCEL SAFETY: `mpsc::UnboundedReceiver::recv` is cancel safe.
          message = control_rx.recv() => {
            match message {
              Some(SourceControlMessage::StreamInfo) => {
                let _ = media_info_tx.send(stream_reader.info.clone());
              },
              Some(SourceControlMessage::StreamState) => {
                // TODO
              },
              None => {
                tracing::error!(%path, "source control channel broke unexpectedly");
                break 'outer;
              },
            };
          },
          // CANCEL SAFETY: `TaskContext::wait_for_stop` is cancel safe.
          _ = task_context.wait_for_stop() => {
            tracing::trace!(%path, "stopping source");
            break 'outer;
          },
        }
      }

      // Before attempting to restart the stream, instruct the existing (broken)
      // one to stop and wait for it to do so.
      stream_reader.stop().await;

      tracing::trace!(%path, "attempting to restart stream");
      'restart: loop {
        match StreamReader::new(&descriptor).await {
          Ok(new_stream_reader) => {
            stream_reader = new_stream_reader;

            // Send reset with new media information to listeners so they can
            // reset their muxers and continue playing.
            let _ = reset_tx.send(stream_reader.info.clone());

            tracing::trace!(%path, "restarted stream");
            break 'restart;
          },
          Err(err) => {
            tracing::error!(
              %err, %descriptor, retry_delay=Self::RETRY_DELAY_SECS,
              "failed to restart stream (waiting before retrying)",
            );
            // We want to wait some time before retrying. We wrap `wait_for_stop` in
            // a timeout to achieve this ...
            match timeout(
              time::Duration::from_secs(Self::RETRY_DELAY_SECS),
              task_context.wait_for_stop(),
            ).await {
              Ok(()) => {
                tracing::trace!(%path, "stopping source (during stream restart)");
                // If `wait_for_stop` returns, we break out of the outer loop and stop ...
                break 'outer;
              },
              Err(_) => {
                // But if the timeout is reached, we simply restart this loop to try and
                // see if we can get the stream reader to work this time.
                continue 'restart;
              },
            }
          },
        }
      }
    }

    stream_reader.stop().await;

    let _ = state_tx.send(SourceState::Stopped(path));
  }

}

pub struct SourceDelegate {
  control_tx: SourceControlTx,
  media_info_rx: SourceMediaInfoRx,
  stream_state_rx: SourceStreamStateRx,
  reset_rx: SourceResetRx,
  packet_rx: SourcePacketRx,
}

impl SourceDelegate {

  pub async fn query_media_info(&mut self) -> Option<media::MediaInfo> {
    if let Ok(()) = self.control_tx.send(SourceControlMessage::StreamInfo) {
      self.media_info_rx.recv().await.ok()
    } else {
      None
    }
  }

  pub async fn query_stream_state(&mut self) -> Option<media::StreamState> {
    if let Ok(()) = self.control_tx.send(SourceControlMessage::StreamState) {
      self.stream_state_rx.recv().await.ok()
    } else {
      None
    }
  }

  pub fn into_parts(self) -> (SourceResetRx, SourcePacketRx) {
    (self.reset_rx, self.packet_rx)
  }

}

pub type SourcePath = String;
pub type SourcePathRef = str;

pub fn normalize_path(path: SourcePath) -> SourcePath {
  if path.starts_with("/") {
    path
  } else {
    format!("/{}", &path)
  }
}