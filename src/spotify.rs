use axum::body::Bytes;
use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::{authentication::Credentials, Error, Session, SessionConfig};
use librespot_playback::{
    audio_backend::{SinkAsBytes, SinkResult},
    config::{AudioFormat, PlayerConfig},
    mixer::{MixerConfig, NoOpVolume},
    player::Player,
    decoder::AudioPacket
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// reminder: cancellation token
/// play spirc stream using provided token. Pipes back to discord module
pub async fn play_stream(access_token: String, cancel_token: CancellationToken) -> impl Stream<Item = Bytes> {
    //stream channel
    let (tx, rx) = mpsc::channel::<Bytes>(64);
    
    let credentials = Credentials::with_token(access_token);
    let session = Session::new(SessionConfig::default(), None);

    // Need a custom backend if I don't want to spawn this as a subprocess.
    // backend pumps the stream as an object we can return instead of calling to a system or returning as stdout
    // Librespot needs docs... 
    let player = Player::new(
        PlayerConfig::default(),
        session.clone(),
        Box::new(NoOpVolume),
        move || Box::new(ParentSink::new(tx.clone())),
    );

    let mixer = mixer::find(None).unwrap();

    // runs the session
    // really hate having no documentation
    let (spirc, spirc_task) = Spirc::new(
        // ConnectConfig is where we name the session. Rebuild custom later
        ConnectConfig::default(),
        session,
        credentials,
        player,
        mixer(MixerConfig::default())?
    );
    tokio::spawn(spirc_task);
    ReceiverStream::new(rx)
}

/// Custom audio_backend for allowing audio stream to be returned as a function output 
pub struct ParentSink {
    tx: mpsc::Sender<Bytes>
}

impl Sink for ParentSink {
    fn write(&mut self, packet: decoder::AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet.samples().map_err(|e| SinkError::OnWrite(e.to_string()))?;
        self.write_bytes(&converter.f64_to_s16(samples).iter())
            .flat_map(|s| s.to_le_bytes)
            .collect::<Vec<u8>>()
    }
}

impl SinkAsBytes for ParentSink {
    fn write_bytes(&mut self, data: &[u8]) -> SinkResult<()> {
        // using blocking send bc channel stream creation is synchronous but receiver is async
        self.tx.blocking_send(Bytes::copy_from_slice(data)).map_err(|_| SinkError::ConnectionRefused("receiver dropped".into()))
    }
}

impl ParentSink {
    pub fn new(tx: mpsc::Sender<Bytes>) -> Self {
        Self {tx}
    }
}