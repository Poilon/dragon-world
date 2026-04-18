//! Footstep sound effects. Two footstep variations are procedurally
//! generated at startup (a filtered noise burst with a low-frequency thump)
//! and played alternately at a cadence tied to the player's horizontal
//! speed.

use std::sync::Arc;

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::player::Player;

#[derive(Resource)]
struct FootstepSources {
    clips: Vec<Handle<AudioSource>>,
}

#[derive(Resource, Default)]
struct FootstepState {
    accumulator: f32,
    cursor: usize,
}

/// Player-facing volume controls, consumed by the music sink and by new
/// footstep playbacks. Edited from the Escape menu (see `menu.rs`).
#[derive(Resource, Clone, Copy)]
pub struct AudioSettings {
    pub music_volume: f32,
    pub sfx_volume: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            music_volume: 0.4,
            sfx_volume: 0.7,
        }
    }
}

/// Marker on the outdoor looping background-music entity.
#[derive(Component)]
struct OutdoorMusic;

/// Marker on the church ambience music entity (plays at volume 0 until
/// the player steps into the nave, then crossfades up).
#[derive(Component)]
struct ChurchMusic;

/// Crossfade state between outdoor music and the church track. `0.0` = fully
/// outdoor, `1.0` = fully inside the church. Exponentially smoothed each
/// frame toward the target dictated by the player's position.
#[derive(Resource, Default)]
struct MusicMix {
    church_mix: f32,
}

/// Church nave footprint in world space (see `spawn_church` in `house.rs`:
/// the church is at (0, -34) with rotation +π/2, half-x=6.0 local becomes
/// half-z=6.0 world, half-z=3.5 local becomes half-x=3.5 world). We inset
/// slightly so the crossfade triggers once the player has actually crossed
/// the threshold, not while brushing past the outside wall.
const CHURCH_MIN_X: f32 = -3.3;
const CHURCH_MAX_X: f32 = 3.3;
const CHURCH_MIN_Z: f32 = -39.8;
const CHURCH_MAX_Z: f32 = -28.2;
/// Exp-smoothing rate for the crossfade. ~4.0 → ~98% complete in 1 s, which
/// feels smooth without being sluggish.
const CHURCH_CROSSFADE_RATE: f32 = 4.0;

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FootstepState>()
            .init_resource::<AudioSettings>()
            .init_resource::<MusicMix>()
            .add_systems(Startup, (build_footsteps, spawn_music))
            .add_systems(Update, (play_footsteps, update_music_mix));
    }
}

/// Read the M4A file from `musics/music1.m4a` at startup and spawn it as
/// a looping AudioPlayer. We bypass `AssetServer::load` because Bevy's
/// AudioLoader doesn't register `.m4a` as a supported extension, but
/// rodio (with `symphonia-aac` + `symphonia-isomp4` features enabled)
/// can decode the raw bytes.
///
/// Certain M4A files trigger an `unreachable!()` panic in rodio's
/// symphonia wrapper (when init returns a SeekError, which rodio wrongly
/// assumes is impossible). We probe the decoder first in
/// `catch_unwind` — if the probe panics, we skip the music entirely so
/// the game still boots.
fn spawn_music(
    mut commands: Commands,
    mut sources: ResMut<Assets<AudioSource>>,
    settings: Res<AudioSettings>,
) {
    // Outdoor theme — audible by default.
    if let Some(handle) = load_music_handle(
        &mut sources,
        &[
            "musics/music1.ogg",
            "musics/music1.wav",
            "musics/music1.mp3",
            "musics/music1.m4a",
        ],
        "outdoor",
    ) {
        commands.spawn((
            AudioPlayer::<AudioSource>(handle),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(settings.music_volume)),
            OutdoorMusic,
            Name::new("OutdoorMusic"),
        ));
    }

    // Church theme — starts silent, faded in by `update_music_mix` when
    // the player steps into the nave.
    if let Some(handle) = load_music_handle(
        &mut sources,
        &[
            "musics/eglise.ogg",
            "musics/eglise.wav",
            "musics/eglise.mp3",
            "musics/eglise.m4a",
        ],
        "church",
    ) {
        commands.spawn((
            AudioPlayer::<AudioSource>(handle),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
            ChurchMusic,
            Name::new("ChurchMusic"),
        ));
    }
}

/// Try each path in order, probing rodio first, then falling back to a
/// symphonia → WAV transcode. Returns a usable `Handle<AudioSource>` or
/// `None` if nothing in the list could be decoded.
fn load_music_handle(
    sources: &mut Assets<AudioSource>,
    candidates: &[&str],
    label: &str,
) -> Option<Handle<AudioSource>> {
    let mut final_bytes: Option<Vec<u8>> = None;
    for path in candidates {
        let Ok(raw) = std::fs::read(path) else {
            continue;
        };
        if probe_rodio(&raw) {
            eprintln!("[audio] music ({label}): loaded {path} (native rodio)");
            final_bytes = Some(raw);
            break;
        }
        eprintln!(
            "[audio] music ({label}): {path} can't be loaded natively — transcoding via symphonia"
        );
        // Pass the file extension as a hint so symphonia picks the right
        // demuxer (e.g. MP3 with an ID3 tag can otherwise get misprobed
        // as AAC and fail with a channel-layout error).
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|s| s.to_str());
        let Some(wav) = transcode_to_wav(&raw, ext) else {
            eprintln!("[audio] music ({label}): symphonia transcode failed for {path}, trying next");
            continue;
        };
        if probe_rodio(&wav) {
            eprintln!("[audio] music ({label}): loaded {path} (symphonia → WAV)");
            final_bytes = Some(wav);
            break;
        } else {
            eprintln!("[audio] music ({label}): transcoded WAV still not accepted, trying next");
        }
    }
    let bytes = final_bytes.or_else(|| {
        eprintln!("[audio] music ({label}): no playable file found. Tried {candidates:?}.");
        None
    })?;
    Some(sources.add(AudioSource {
        bytes: Arc::from(bytes.into_boxed_slice()),
    }))
}

/// Return true if rodio can decode `bytes` without panicking. The panic
/// hook is muted during the probe so expected failures don't spam
/// stderr with `thread panicked at ...`.
fn probe_rodio(bytes: &[u8]) -> bool {
    let owned = bytes.to_vec();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rodio::Decoder::new(std::io::Cursor::new(owned)).is_ok()
    }));
    std::panic::set_hook(prev_hook);
    matches!(result, Ok(true))
}

/// Decode a compressed audio blob (M4A/AAC, MP3, OGG, etc.) with
/// symphonia and re-encode the PCM as a self-contained 16-bit WAV file
/// in memory. The result can be handed to rodio's WAV decoder which is
/// far more tolerant than its generic symphonia wrapper.
fn transcode_to_wav(src_bytes: &[u8], ext_hint: Option<&str>) -> Option<Vec<u8>> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error as SymErr;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let cursor = std::io::Cursor::new(src_bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }
    let probed = match symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[audio] transcode: probe failed: {e}");
            return None;
        }
    };
    let mut format = probed.format;

    let track = match format.default_track() {
        Some(t) => t.clone(),
        None => {
            eprintln!("[audio] transcode: no default track");
            return None;
        }
    };
    let track_id = track.id;

    let mut decoder = match symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
    {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[audio] transcode: no decoder for codec: {e}");
            return None;
        }
    };

    // `codec_params.channels` is often missing for AAC-in-M4A; we
    // derive the real `SignalSpec` from the first decoded packet
    // instead.
    let mut spec: Option<symphonia::core::audio::SignalSpec> = None;
    let mut pcm: Vec<i16> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<i16>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(_)) => break,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_buf.is_none() {
                    let s = *decoded.spec();
                    spec = Some(s);
                    sample_buf =
                        Some(SampleBuffer::<i16>::new(decoded.capacity() as u64, s));
                }
                let buf = sample_buf.as_mut()?;
                buf.copy_interleaved_ref(decoded);
                pcm.extend_from_slice(buf.samples());
            }
            Err(SymErr::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    if pcm.is_empty() {
        eprintln!("[audio] transcode: no PCM samples decoded");
        return None;
    }
    let spec = spec?;
    let sample_rate = spec.rate;
    let channel_count = spec.channels.count() as u16;
    eprintln!(
        "[audio] transcode: decoded {} samples @ {sample_rate} Hz × {channel_count}ch → WAV",
        pcm.len()
    );
    Some(build_wav(&pcm, sample_rate, channel_count))
}

fn build_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    const BITS: u16 = 16;
    let byte_rate = sample_rate * channels as u32 * (BITS / 8) as u32;
    let block_align = channels * (BITS / 8);
    let data_size = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Drive the crossfade between the outdoor and church music tracks based
/// on the player's position, and push the resulting per-sink volumes
/// (scaled by the master `AudioSettings.music_volume`) every frame.
///
/// Running this every frame (rather than only on change) keeps the
/// crossfade responsive to both player movement and slider changes
/// without needing a separate system.
fn update_music_mix(
    time: Res<Time>,
    settings: Res<AudioSettings>,
    mut mix: ResMut<MusicMix>,
    player_q: Query<&Transform, With<Player>>,
    mut outdoor_q: Query<&mut AudioSink, (With<OutdoorMusic>, Without<ChurchMusic>)>,
    mut church_q: Query<&mut AudioSink, (With<ChurchMusic>, Without<OutdoorMusic>)>,
) {
    // Decide the target mix from the player's current position.
    let target = if let Ok(tf) = player_q.single() {
        let p = tf.translation;
        let inside_church = p.x >= CHURCH_MIN_X
            && p.x <= CHURCH_MAX_X
            && p.z >= CHURCH_MIN_Z
            && p.z <= CHURCH_MAX_Z;
        if inside_church { 1.0 } else { 0.0 }
    } else {
        0.0
    };

    let alpha = 1.0 - (-CHURCH_CROSSFADE_RATE * time.delta_secs()).exp();
    mix.church_mix += (target - mix.church_mix) * alpha;
    // Snap to exact endpoints once the lerp is essentially done, otherwise
    // the church sink sits at a tiny non-zero volume forever.
    if mix.church_mix < 1e-3 {
        mix.church_mix = 0.0;
    } else if mix.church_mix > 1.0 - 1e-3 {
        mix.church_mix = 1.0;
    }

    let master = settings.music_volume.clamp(0.0, 1.0);
    let outdoor_vol = master * (1.0 - mix.church_mix);
    let church_vol = master * mix.church_mix;

    for mut sink in &mut outdoor_q {
        sink.set_volume(Volume::Linear(outdoor_vol));
    }
    for mut sink in &mut church_q {
        sink.set_volume(Volume::Linear(church_vol));
    }
}

fn build_footsteps(
    mut commands: Commands,
    mut sources: ResMut<Assets<AudioSource>>,
) {
    // Three variations seeded differently so consecutive steps don't
    // sound identical.
    let clips = [101u64, 271, 457]
        .into_iter()
        .map(|seed| {
            let wav = synth_footstep_wav(seed);
            sources.add(AudioSource {
                bytes: Arc::from(wav.into_boxed_slice()),
            })
        })
        .collect();
    commands.insert_resource(FootstepSources { clips });
}

/// Simple stochastic FIR/IIR synth for a "thud on dirt" footstep. The
/// output is a valid 16-bit PCM mono WAV file that bevy_audio can decode
/// natively (via the `wav` feature).
fn synth_footstep_wav(seed: u64) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 44_100;
    const DURATION_SECS: f32 = 0.22;
    let num_samples = (SAMPLE_RATE as f32 * DURATION_SECS) as usize;

    // LCG PRNG for deterministic per-seed noise.
    let mut rng = seed.wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let mut noise = || -> f32 {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // Map high bits to [-1, 1].
        (rng >> 33) as i32 as f32 / i32::MAX as f32
    };

    // Two cascaded one-pole low-passes smooth the noise down to a dull
    // thump. The thump oscillator adds a ~90 Hz body kick.
    let mut lp1 = 0.0_f32;
    let mut lp2 = 0.0_f32;
    let lp_alpha = 0.08;

    let mut samples: Vec<i16> = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f32 / SAMPLE_RATE as f32;
        // Fast exponential decay — footsteps are short.
        let envelope = (-t * 16.0).exp();
        // Low-frequency thump on the first 40 ms.
        let thump_env = (-t * 40.0).exp();
        let thump = (t * std::f32::consts::TAU * 95.0).sin() * thump_env * 0.7;
        // Filtered noise for the "scuff".
        let n = noise();
        lp1 = lp1 * (1.0 - lp_alpha) + n * lp_alpha;
        lp2 = lp2 * (1.0 - lp_alpha) + lp1 * lp_alpha;
        let scuff = lp2 * 1.8;
        let sample = (scuff + thump) * envelope * 0.55;
        samples.push((sample.clamp(-1.0, 1.0) * 32_000.0) as i16);
    }

    // PCM mono 16-bit WAV.
    let data_size = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    bytes
}

/// Per-frame: if the player is moving on the ground, accumulate time and
/// emit a footstep when enough has passed. Cadence scales with speed so
/// running produces faster steps than walking.
fn play_footsteps(
    time: Res<Time>,
    mut commands: Commands,
    mut state: ResMut<FootstepState>,
    sources: Option<Res<FootstepSources>>,
    settings: Res<AudioSettings>,
    player_q: Query<&Player>,
) {
    let Some(sources) = sources else { return };
    let Ok(player) = player_q.single() else {
        return;
    };

    if !player.grounded || player.horizontal_speed < 0.4 {
        state.accumulator = 0.0;
        return;
    }

    let speed_norm = (player.horizontal_speed / 5.5).clamp(0.8, 2.0);
    let interval = 0.42 / speed_norm;
    state.accumulator += time.delta_secs();
    while state.accumulator >= interval {
        state.accumulator -= interval;
        let clip = sources.clips[state.cursor % sources.clips.len()].clone();
        state.cursor = state.cursor.wrapping_add(1);
        let vol = (0.6 * settings.sfx_volume).clamp(0.0, 1.0);
        commands.spawn((
            AudioPlayer(clip),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(vol)),
        ));
    }
}
