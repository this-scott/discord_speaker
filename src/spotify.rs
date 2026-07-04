use librespot::{playback::{config::AudioFormat, mixer::NoOpVolume}, protocol::credentials};
use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::{authentication::Credentials, Error, Session, SessionConfig};
use librespot_playback::{
    audio_backend, mixer,
    config::{AudioFormat, PlayerConfig},
    mixer::{MixerConfig, NoOpVolume},
    player::Player
};
use tokio_util::sync::CancellationToken;

// reminder: cancellation token
/// play spirc stream using provided token. Pipes back to discord module
pub async fn play_stream(access_token: String, cancel_token: CancellationToken) -> Result<(), Error> {
    let credentials = Credentials::with_token(access_token);
    let session = Session::new(SessionConfig::default(), None);

    let backend = audio_backend::find(Some("pipe".to_string())).unwrap();

    // Need a custom backend if I don't want to spawn this as a subprocess.
    // backend pumps the stream as an object we can return instead of calling to a system or returning as stdout
    // Librespot needs docs... 
    let player = Player::new(
        PlayerConfig::default(),
        session.clone(),
        Box::new(NoOpVolume),
        move || {
            let format = AudioFormat::default();
            let device = None;
            backend(device,format)
        },
    );

    let mixer = mixer::find(None).unwrap();

    // runs the session
    let (spirc, spirc_task): (Spric, _) = Spirc::new(
        ConnectConfig::default(),
        session,
        credentials,
        player,
        mixer(MixerConfig::default())?
    ).await?;

    Ok(())
}