// cargo xtask bundle rust_nam --release

const INPUT_GAIN_MIN: f32 = -30.0;
const INPUT_GAIN_MAX: f32 = 30.0;
const INPUT_GAIN_DEF: f32 = 0.0;

const GATE_ENABLED_DEF: bool = true;

const GATE_THRESHOLD_MIN: f32 = -80.0;
const GATE_THRESHOLD_MAX: f32 = 0.0;
const GATE_THRESHOLD_DEF: f32 = -60.0;

const GATE_RELEASE_MIN: f32 = 10.0;
const GATE_RELEASE_MAX: f32 = 500.0;
const GATE_RELEASE_DEF: f32 = 100.0;

const GATE_ATTACK_MIN: f32 = 0.1;
const GATE_ATTACK_MAX: f32 = 50.0;
const GATE_ATTACK_DEFAULT: f32 = 5.0;

const OUTPUT_GAIN_MIN: f32 = -30.0;
const OUTPUT_GAIN_MAX: f32 = 30.0;
const OUTPUT_GAIN_DEF: f32 = 0.0;

use nih_plug::prelude::*;
use std::sync::{Arc};

struct RustNam {
    params: Arc<RustNamParams>,
    rms_state: f32, // running RMS estimate
    gain_reduction: f32, // current gate gain, 0.0 = closed, 1.0 = open
    sample_rate: f32, // needed to convert ms to per-sample coefficients
}

/// The [`Params`] derive macro gathers all the information needed for the wrapper to know about
/// the plugin's parameters, persistent serializable fields, and nested parameter groups. You can
/// also easily implement [`Params`] by hand if you want to, for instance, have multiple instances
/// of a parameters struct for multiple identical oscillators/filters/envelopes.
#[derive(Params)]
struct RustNamParams {
    /// The parameter's ID is used to identify the parameter in the wrapped plugin API. As long as
    /// these IDs remain constant, you can rename and reorder these fields as you wish. The
    /// parameters are exposed to the host in the same order they were defined. In this case, this
    /// gain parameter is stored as linear gain while the values are displayed in decibels.
    #[id = "input_gain"]
    pub input_gain: FloatParam,

    #[id = "gate_enabled"]
    pub gate_enabled: BoolParam,

    #[id = "gate_threshold"]
    pub gate_threshold: FloatParam,

    #[id = "gate_release"]
    pub gate_release: FloatParam,

    #[id = "gate_attack"]
    pub gate_attack: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,
}

impl Default for RustNam {
    fn default() -> Self {
        Self {
            params: Arc::new(RustNamParams::default()),
            rms_state: 0.0,
            gain_reduction: 0.0,
            sample_rate: 44100.0, // overwritten in initialize()
        }
    }
}

impl Default for RustNamParams {
    fn default() -> Self {
        Self {
            // *** INPUT GAIN ***
            // This gain is stored as linear gain. NIH-plug comes with useful conversion functions
            // to treat these kinds of parameters as if we were dealing with decibels. Storing this
            // as decibels is easier to work with, but requires a conversion for every sample.
            input_gain: FloatParam::new(
                "Input Gain",
                util::db_to_gain(INPUT_GAIN_DEF),
                FloatRange::Skewed {
                    min: util::db_to_gain(INPUT_GAIN_MIN),
                    max: util::db_to_gain(INPUT_GAIN_MAX),
                    // This makes the range appear as if it was linear when displaying the values as
                    // decibels
                    factor: FloatRange::gain_skew_factor(INPUT_GAIN_MIN, INPUT_GAIN_MAX),
                },
            )
            // Because the gain parameter is stored as linear gain instead of storing the value as
            // decibels, we need logarithmic smoothing
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            // There are many predefined formatters we can use here. If the gain was stored as
            // decibels instead of as a linear gain value, we could have also used the
            // `.with_step_size(0.1)` function to get internal rounding.
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            // *** ENABLE GATE ***

            gate_enabled: BoolParam::new(
                "Enable Gate",
                GATE_ENABLED_DEF
            ),

            // *** GATE THRESHOLD ***

            gate_threshold: FloatParam::new(
                "Gate Threshold",
                util::db_to_gain(GATE_THRESHOLD_DEF),
                FloatRange::Skewed {
                    min: util::db_to_gain(GATE_THRESHOLD_MIN),
                    max: util::db_to_gain(GATE_THRESHOLD_MAX),
                    factor: FloatRange::gain_skew_factor(GATE_THRESHOLD_MIN, GATE_THRESHOLD_MAX),
                },
            )
                .with_smoother(SmoothingStyle::Logarithmic(50.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
                .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            // *** GATE RELEASE ***

            gate_release: FloatParam::new(
                "Gate Release",
                GATE_RELEASE_DEF,
                FloatRange::Skewed {
                    min: GATE_RELEASE_MIN,
                    max: GATE_RELEASE_MAX,
                    factor : FloatRange::skew_factor(0.5)
                },
            )
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
                .with_string_to_value(Arc::new(|s| s.parse().ok())),

            // *** GATE ATTACK ***

            gate_attack: FloatParam::new(
                "Gate Attack",
                GATE_ATTACK_DEFAULT,
                FloatRange::Skewed {
                    min: GATE_ATTACK_MIN,
                    max: GATE_ATTACK_MAX,
                    factor : FloatRange::skew_factor(0.5)
                },
            )
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
                .with_string_to_value(Arc::new(|s| s.parse().ok())),

            // *** OUTPUT GAIN ***

            output_gain: FloatParam::new(
                "Output Gain",
                util::db_to_gain(OUTPUT_GAIN_DEF),
                FloatRange::Skewed {
                    min: util::db_to_gain(OUTPUT_GAIN_MIN),
                    max: util::db_to_gain(OUTPUT_GAIN_MAX),
                    factor: FloatRange::gain_skew_factor(OUTPUT_GAIN_MIN, OUTPUT_GAIN_MAX),
                },
            )
                .with_smoother(SmoothingStyle::Logarithmic(50.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
                .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl Plugin for RustNam {
    const NAME: &'static str = "Rust NAM";
    const VENDOR: &'static str = "Aidan O'Brien";
    // You can use `env!("CARGO_PKG_HOMEPAGE")` to reference the homepage field from the
    // `Cargo.toml` file here
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "aidobr-5@student.ltu.se";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // The first audio IO layout is used as the default. The other layouts may be selected either
    // explicitly or automatically by the host or the user depending on the plugin API/backend.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(1),

        aux_input_ports: &[],
        aux_output_ports: &[],

        // Individual ports and the layout as a whole can be named here. By default, these names
        // are generated as needed. This layout will be called 'Mono', while a layout with
        // two input and output channels would be called 'Stereo'.
        names: PortNames::const_default(),
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    // Setting this to `true` will tell the wrapper to split the buffer up into smaller blocks
    // whenever there are inter-buffer parameter changes. This way no changes to the plugin are
    // required to support sample accurate automation and the wrapper handles all the boring
    // stuff like making sure transport and other timing information stays consistent between the
    // splits.
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    // If the plugin can send or receive SysEx messages, it can define a type to wrap around those
    // messages here. The type implements the `SysExMessage` trait, which allows conversion to and
    // from plain byte buffers.
    type SysExMessage = ();
    // More advanced plugins can use this to run expensive background tasks. See the field's
    // documentation for more information. `()` means that the plugin does not have any background
    // tasks.
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    // This plugin doesn't need any special initialization, but if you need to do anything expensive
    // then this would be the place. State is kept around when the host reconfigures the
    // plugin. If we do need special initialization, we could implement the `initialize()` and/or
    // `reset()` methods
    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        // Resize buffers and perform other potentially expensive initialization operations here.
        // The `reset()` function is always called right after this function. You can remove this
        // function if you do not need it.
        self.sample_rate = buffer_config.sample_rate;
        true
    }

    fn reset(&mut self) {
        // Reset buffers and envelopes here. This can be called from the audio thread and may not
        // allocate. You can remove this function if you do not need it.
        self.rms_state = 0.0;
        self.gain_reduction = 0.0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_samples() {
            // Smoothing is optionally built into the parameters themselves
            let input_gain = self.params.input_gain.smoothed.next();

            for sample in channel_samples {
                *sample *= input_gain;
            }
        }

        ProcessStatus::Normal
    }

    // This can be used for cleaning up special resources like socket connections whenever the
    // plugin is deactivated. Most plugins won't need to do anything here.
    fn deactivate(&mut self) {}
}

impl ClapPlugin for RustNam {
    const CLAP_ID: &'static str = "rust_nam_clap";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A NAM plugin implemented with Rust.");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;

    // Don't forget to change these features
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Mono];
}

impl Vst3Plugin for RustNam {
    const VST3_CLASS_ID: [u8; 16] = *b"RustNamVstThree!";

    // And also don't forget to change these categories
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nih_export_clap!(RustNam);
nih_export_vst3!(RustNam);