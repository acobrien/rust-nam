// cargo xtask bundle rust_nam --release

const INPUT_GAIN_MIN: f32 = -30.0;
const INPUT_GAIN_MAX: f32 = 30.0;
const INPUT_GAIN_DEF: f32 = 0.0;

const GATE_ENABLED_DEF: bool = true;

const GATE_THRESHOLD_MIN: f32 = -80.0;
const GATE_THRESHOLD_MAX: f32 = 0.0;
const GATE_THRESHOLD_DEF: f32 = -40.0;

const GATE_ATTACK_MIN: f32 = 0.1;
const GATE_ATTACK_MAX: f32 = 50.0;
const GATE_ATTACK_DEF: f32 = 2.0;

const GATE_RELEASE_MIN: f32 = 50.0;
const GATE_RELEASE_MAX: f32 = 500.0;
const GATE_RELEASE_DEF: f32 = 200.0;

const GATE_DECAY_MIN: f32 = 1.0;
const GATE_DECAY_MAX: f32 = 50.0;
const GATE_DECAY_DEF: f32 = 10.0;

const OUTPUT_GAIN_MIN: f32 = -30.0;
const OUTPUT_GAIN_MAX: f32 = 30.0;
const OUTPUT_GAIN_DEF: f32 = 0.0;

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

struct RustNam {
    params: Arc<RustNamParams>,
    egui_state: Arc<EguiState>,
    rms_sq: f32, // running mean-square estimate
    gate_openness: f32, // 0.0 to 1.0, closed to open
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

    // Time for gate to open ~63% when above threshold
    #[id = "gate_attack"]
    pub gate_attack: FloatParam,

    // Time for gate to close ~63% when below threshold
    #[id = "gate_release"]
    pub gate_release: FloatParam,

    // Time for ~63% of a step change in signal power to be reflected in RMS
    #[id = "gate_rms_decay"]
    pub gate_decay: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,
}

impl Default for RustNam {
    fn default() -> Self {
        Self {
            params: Arc::new(RustNamParams::default()),
            egui_state: EguiState::from_size(300, 200),
            rms_sq: 0.0,
            gate_openness: 0.0,
            sample_rate: 44100.0, // overwritten in initialize()
        }
    }
}

struct EditorState {
    settings_open: bool,
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

            // *** GATE ATTACK ***

            gate_attack: FloatParam::new(
                "Gate Attack",
                GATE_ATTACK_DEF,
                FloatRange::Skewed {
                    min: GATE_ATTACK_MIN,
                    max: GATE_ATTACK_MAX,
                    factor: FloatRange::skew_factor(0.5)
                },
            )
                .with_smoother(SmoothingStyle::Linear(50.0))
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
                .with_string_to_value(Arc::new(|s| s.parse().ok())),

            // *** GATE RELEASE ***

            gate_release: FloatParam::new(
                "Gate Release",
                GATE_RELEASE_DEF,
                FloatRange::Skewed {
                    min: GATE_RELEASE_MIN,
                    max: GATE_RELEASE_MAX,
                    factor: FloatRange::skew_factor(0.5)
                },
            )
                .with_smoother(SmoothingStyle::Linear(50.0))
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
                .with_string_to_value(Arc::new(|s| s.parse().ok())),

            // *** GATE DECAY ***

            gate_decay: FloatParam::new(
                "Gate Decay",
                GATE_DECAY_DEF,
                FloatRange::Skewed {
                    min: GATE_DECAY_MIN,
                    max: GATE_DECAY_MAX,
                    factor: FloatRange::skew_factor(0.5)
                },
            )
                .with_smoother(SmoothingStyle::Linear(50.0))
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

fn drag_slider<P: Param>(ui: &mut egui::Ui, param: &P, setter: &ParamSetter) {
    let height = 20.0;
    let handle_width = 14.0;
    let value_label_width = 65.0;

    ui.horizontal(|ui| {
        let slider_width = (ui.available_width() - value_label_width - ui.spacing().item_spacing.x).max(50.0);
        let drag_range = (slider_width - handle_width).max(1.0);

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(slider_width, height),
            egui::Sense::drag(),
        );

        let normalized = param.unmodulated_normalized_value();

        if response.drag_started() {
            setter.begin_set_parameter(param);
        }
        if response.dragged() {
            let new_normalized = (normalized + response.drag_delta().x / drag_range).clamp(0.0, 1.0);
            setter.set_parameter(param, param.preview_plain(new_normalized));
        }
        if response.drag_stopped() {
            setter.end_set_parameter(param);
        }

        if ui.is_rect_visible(rect) {
            // Dark rail
            ui.painter().rect_filled(rect, 3.0, egui::Color32::from_gray(35));

            // Blue fill from left to center of handle
            let handle_center_x = (rect.left() + normalized * drag_range + handle_width / 2.0).min(rect.right());
            let fill_rect = egui::Rect::from_min_max(rect.min, egui::pos2(handle_center_x, rect.bottom()));
            ui.painter().rect_filled(
                fill_rect,
                egui::CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 },
                egui::Color32::from_rgb(50, 110, 190),
            );

            // Grey handle rect
            let handle_x = rect.left() + normalized * drag_range;
            let handle_rect = egui::Rect::from_min_size(
                egui::pos2(handle_x, rect.top() + 1.0),
                egui::vec2(handle_width, height - 2.0),
            );
            let handle_color = if response.dragged() {
                egui::Color32::from_gray(210)
            } else if response.hovered() {
                egui::Color32::from_gray(175)
            } else {
                egui::Color32::from_gray(140)
            };
            ui.painter().rect_filled(handle_rect, 2.0, handle_color);

            // Param name centered on rail
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                param.name(),
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(220),
            );
        }

        let _ = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

        // Current value outside the slider
        let value_str = param.normalized_value_to_string(param.unmodulated_normalized_value(), true);
        ui.label(value_str);
    });
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

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let egui_state = self.egui_state.clone();

        create_egui_editor(
            egui_state,
            EditorState { settings_open: false },
            |_ctx, _state| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading("Rust NAM");
                    ui.separator();
                    drag_slider(ui, &params.input_gain, setter);
                    drag_slider(ui, &params.output_gain, setter);
                    ui.separator();
                    ui.horizontal(|ui| {
                        let mut enabled = params.gate_enabled.value();
                        if ui.checkbox(&mut enabled, "Noise Gate").changed() {
                            setter.begin_set_parameter(&params.gate_enabled);
                            setter.set_parameter(&params.gate_enabled, enabled);
                            setter.end_set_parameter(&params.gate_enabled);
                        }
                        if ui.button("Settings").clicked() {
                            state.settings_open = !state.settings_open;
                        }
                    });
                });

                egui::Window::new("Gate Settings")
                    .open(&mut state.settings_open)
                    .resizable(false)
                    .show(ctx, |ui| {
                        drag_slider(ui, &params.gate_threshold, setter);
                        drag_slider(ui, &params.gate_attack, setter);
                        drag_slider(ui, &params.gate_release, setter);
                        drag_slider(ui, &params.gate_decay, setter);
                    });
            },
        )
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

    // Reset buffers and envelopes here. This can be called from the audio thread and may not
    // allocate. You can remove this function if you do not need it.
    fn reset(&mut self) {
        self.rms_sq = 0.0;
        self.gate_openness = 0.0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let gate_enabled = self.params.gate_enabled.value();

        for channel_samples in buffer.iter_samples() {
            // These values must be inside the loop because they progress through the smoothed set
            let input_gain = self.params.input_gain.smoothed.next();
            let threshold = self.params.gate_threshold.smoothed.next();
            let output_gain = self.params.output_gain.smoothed.next();

            // The below values may not necessarily require smoothing
            let rms_pole = (-1.0 / (self.params.gate_decay.smoothed.next() * 0.001 * self.sample_rate)).exp();
            let attack_pole = (-1.0 / (self.params.gate_attack.smoothed.next() * 0.001 * self.sample_rate)).exp();
            let release_pole = (-1.0 / (self.params.gate_release.smoothed.next() * 0.001 * self.sample_rate)).exp();

            for sample in channel_samples {
                *sample *= input_gain; // Apply input gain

                if gate_enabled { // Apply gating
                    self.rms_sq = rms_pole * self.rms_sq + (1.0 - rms_pole) * (*sample * *sample);
                    let rms = self.rms_sq.sqrt();

                    let target = if rms >= threshold { 1.0 } else { 0.0 };
                    let gate_speed = if target > self.gate_openness { attack_pole } else { release_pole };
                    self.gate_openness = gate_speed * self.gate_openness + (1.0 - gate_speed) * target;

                    *sample *= self.gate_openness;
                }

                // TODO: Apply dummy NAM transformation

                *sample *= output_gain; // Apply output gain
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