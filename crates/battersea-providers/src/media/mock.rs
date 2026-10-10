use super::{
    read_string_option, read_u64_option, MediaBackendConfig, MediaGenerationActivityReporter,
    MediaGenerationActivityUpdate, MediaGenerationAdapter, MediaGenerationBatchReporter,
    MediaRenderRequest, MediaRenderResult, MediaTimingRecorder,
};
use crate::adapter::error::EngineAdapterRequestError;
use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use async_trait::async_trait;
use base64::Engine as _;
use battersea_model::media::ControllerActivityState;
use battersea_model::media::{MediaAsset, MediaKind, MediaRenderType};
use png::{BitDepth, ColorType, Encoder};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) fn create_mock_media_generator(
    backend: MediaBackendConfig,
) -> Result<Arc<dyn MediaGenerationAdapter>, EngineAdapterRequestError> {
    let configured_url = backend
        .options
        .extra
        .get("mockUrl")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let prompt_preview_font = if matches!(
        backend.capability,
        battersea_model::media::MediaCapability::ImageGeneration
    ) && configured_url.is_none()
    {
        Some(load_mock_prompt_font(&backend)?)
    } else {
        None
    };

    Ok(Arc::new(MockMediaGenerator {
        backend,
        configured_url,
        prompt_preview_font,
    }))
}

struct MockMediaGenerator {
    backend: MediaBackendConfig,
    configured_url: Option<String>,
    prompt_preview_font: Option<FontArc>,
}

#[async_trait]
impl MediaGenerationAdapter for MockMediaGenerator {
    fn backend(&self) -> &MediaBackendConfig {
        &self.backend
    }

    fn prepare(
        &self,
        request: MediaRenderRequest,
    ) -> Result<super::PreparedMediaRequest, EngineAdapterRequestError> {
        super::prepare_media_request(self.backend(), request)
    }

    async fn submit(
        &self,
        prepared: super::PreparedMediaRequest,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        batch_reporter: Option<Arc<dyn super::MediaGenerationBatchReporter>>,
    ) -> Result<super::MediaSubmission, EngineAdapterRequestError> {
        self.render(prepared, cancellation, activity_reporter, batch_reporter)
            .await
            .map(super::MediaSubmission::Complete)
    }
}
impl MockMediaGenerator {
    async fn render(
        &self,
        prepared: super::PreparedMediaRequest,
        cancellation: CancellationToken,
        activity_reporter: Option<Arc<dyn MediaGenerationActivityReporter>>,
        batch_reporter: Option<Arc<dyn MediaGenerationBatchReporter>>,
    ) -> Result<MediaRenderResult, EngineAdapterRequestError> {
        let request = prepared.render_input.clone();
        if let Some(reporter) = &activity_reporter {
            reporter
                .report_activity(MediaGenerationActivityUpdate {
                    state: ControllerActivityState::Working,
                    event: None,
                    message: String::new(),
                    provider_job_id: None,
                    error_code: None,
                    slot_id: None,
                    slot_index: None,
                    progress: None,
                    eta_ms: None,
                    preview_asset: None,
                    partial_index: None,
                })
                .await?;
        }

        // Mock renders never touch a network, but provenance rows
        // from them should still carry a faithful, deterministic
        // "envelope" so downstream tooling (and tests) sees the same
        // shape as a real provider. The request envelope mirrors the
        // resolved render request plus the mock-specific knobs; the
        // response envelope is filled in just before we return.
        let provider_request = prepared.body;
        let mut timing = MediaTimingRecorder::start();
        let phase_started_at = chrono::Utc::now();
        let phase_started_instant = std::time::Instant::now();
        let sequence = self
            .backend
            .options
            .extra
            .get("mockSequence")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let target_asset_count = mock_asset_count(&request);
        if sequence.is_empty() {
            let delay_ms = read_u64_option(&self.backend.options.extra, "mockDelayMs", 0);
            // Emit per-index progress ticks before completion so the engine's
            // slot event bridge can attribute provider-sourced progress to each
            // placeholder. By default the ticks are spread across
            // `mockDelayMs`; set `mockProgressDurationMs` to override that, or
            // `mockProgressTicks: false` to suppress them entirely.
            let mock_progress_ticks = self
                .backend
                .options
                .extra
                .get("mockProgressTicks")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if mock_progress_ticks && activity_reporter.is_some() {
                let progress_duration_ms =
                    read_mock_progress_duration_ms(&self.backend.options.extra, delay_ms);
                let progress_step_count =
                    read_mock_progress_step_count(&self.backend.options.extra);
                emit_mock_progress_ticks(
                    &activity_reporter,
                    target_asset_count,
                    progress_duration_ms,
                    progress_step_count,
                    &cancellation,
                )
                .await?;
                if delay_ms > progress_duration_ms {
                    sleep_with_cancellation(delay_ms - progress_duration_ms, &cancellation).await?;
                }
            } else {
                sleep_with_cancellation(delay_ms, &cancellation).await?;
            }
            let assets: Vec<MediaAsset> = (0..target_asset_count)
                .map(|index| build_mock_asset(self, &request, None, index))
                .collect();
            let provider_response = build_mock_response_envelope("mock-job", &assets);
            timing.record_phase(
                "mock render",
                "provider-call",
                true,
                phase_started_at,
                phase_started_instant,
            );
            return Ok(MediaRenderResult {
                provider_job_id: Some("mock-job".to_string()),
                assets,
                provider_request: Some(provider_request),
                provider_response: Some(provider_response),
                timing: Some(timing.finish_client_estimate()),
            });
        }

        let mut all_assets = Vec::new();
        for step in sequence.iter() {
            if all_assets.len() >= target_asset_count {
                break;
            }
            let delay_ms = step
                .get("delayMs")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_else(|| read_u64_option(&self.backend.options.extra, "mockDelayMs", 0));
            sleep_with_cancellation(delay_ms, &cancellation).await?;
            let remaining = target_asset_count.saturating_sub(all_assets.len());
            let mut assets = step
                .get("frames")
                .and_then(serde_json::Value::as_array)
                .map(|frames| {
                    frames
                        .iter()
                        .enumerate()
                        .map(|(frame_index, frame)| {
                            build_mock_asset(
                                self,
                                &request,
                                Some(frame),
                                all_assets.len() + frame_index,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| {
                    vec![build_mock_asset(
                        self,
                        &request,
                        Some(step),
                        all_assets.len(),
                    )]
                });
            assets.truncate(remaining);
            all_assets.extend(assets.clone());
            if let Some(reporter) = &batch_reporter {
                reporter
                    .report_batch(MediaRenderResult::without_envelopes(
                        Some("mock-job".to_string()),
                        assets,
                    ))
                    .await?;
            }
        }
        if all_assets.len() < target_asset_count {
            let assets = (all_assets.len()..target_asset_count)
                .map(|index| build_mock_asset(self, &request, None, index))
                .collect::<Vec<_>>();
            all_assets.extend(assets.clone());
            if let Some(reporter) = &batch_reporter {
                reporter
                    .report_batch(MediaRenderResult::without_envelopes(
                        Some("mock-job".to_string()),
                        assets,
                    ))
                    .await?;
            }
        }

        let provider_response = build_mock_response_envelope("mock-job", &all_assets);
        timing.record_phase(
            "mock render",
            "provider-call",
            true,
            phase_started_at,
            phase_started_instant,
        );
        Ok(MediaRenderResult {
            provider_job_id: Some("mock-job".to_string()),
            assets: all_assets,
            provider_request: Some(provider_request),
            provider_response: Some(provider_response),
            timing: Some(timing.finish_client_estimate()),
        })
    }
}

/// Builds the deterministic stand-in "request body" recorded for a
/// mock render. Mirrors the resolved render request plus the
/// mock-specific options that influenced the output, so a provenance
/// row from the mock backend is just as inspectable as a real one.
pub(crate) fn build_mock_request_envelope(
    backend: &MediaBackendConfig,
    request: &MediaRenderRequest,
) -> serde_json::Value {
    let mut envelope = serde_json::json!({
        "backend": "mock",
        "kind": match request.kind {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
        },
        "prompt_text": request.prompt_text,
        "negative_prompt": request.negative_prompt,
        "options": request.options,
        "references": request.references,
    });
    let map = envelope
        .as_object_mut()
        .expect("mock request envelope is an object");
    for key in [
        "mockSequence",
        "mockUrl",
        "mockDelayMs",
        "mockProgressTicks",
        "mockProgressDurationMs",
        "mockProgressStepCount",
    ] {
        if let Some(value) = backend.options.extra.get(key) {
            map.insert(key.to_string(), value.clone());
        }
    }
    if let Some(url) = backend
        .options
        .extra
        .get("mockUrl")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        map.insert(
            "configuredUrl".to_string(),
            serde_json::Value::String(url.to_string()),
        );
    }
    envelope
}

/// Builds the deterministic stand-in "response body" recorded for a
/// mock render.
fn build_mock_response_envelope(provider_job_id: &str, assets: &[MediaAsset]) -> serde_json::Value {
    serde_json::json!({
        "provider_job_id": provider_job_id,
        "asset_count": assets.len(),
        "assets": assets,
    })
}

fn mock_asset_count(request: &MediaRenderRequest) -> usize {
    if matches!(request.kind, MediaKind::Image) {
        usize::from(request.options.count.unwrap_or(1).max(1))
    } else {
        1
    }
}

fn read_mock_progress_duration_ms(
    options: &std::collections::BTreeMap<String, serde_json::Value>,
    delay_ms: u64,
) -> u64 {
    if options.contains_key("mockProgressDurationMs") {
        read_u64_option(options, "mockProgressDurationMs", delay_ms)
    } else {
        delay_ms
    }
}

fn read_mock_progress_step_count(
    options: &std::collections::BTreeMap<String, serde_json::Value>,
) -> u64 {
    read_u64_option(options, "mockProgressStepCount", 3).clamp(1, 100)
}

async fn sleep_with_cancellation(
    delay_ms: u64,
    cancellation: &CancellationToken,
) -> Result<(), crate::EngineAdapterRequestError> {
    if delay_ms == 0 {
        if cancellation.is_cancelled() {
            return Err(EngineAdapterRequestError::new(
                "mock",
                "Media generation cancelled.",
                "cancelled",
            ));
        }
        return Ok(());
    }
    tokio::select! {
        _ = cancellation.cancelled() => {
            Err(EngineAdapterRequestError::new("mock", "Media generation cancelled.", "cancelled"))
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => Ok(())
    }
}

/// Emits per-index progress updates so the engine's slot event bridge
/// switches each placeholder to provider-sourced progress.
///
/// The configured tick count is emitted for every requested slot, spread
/// evenly over `duration_ms`. Explicit `duration_ms = 0` makes the loop emit
/// instantly — pianola sees the events without sleeping. The 0.9 cap leaves
/// the snap-to-1.0 to the engine's `slot.completed` emission.
async fn emit_mock_progress_ticks(
    activity_reporter: &Option<Arc<dyn MediaGenerationActivityReporter>>,
    slot_count: usize,
    duration_ms: u64,
    step_count: u64,
    cancellation: &CancellationToken,
) -> Result<(), crate::EngineAdapterRequestError> {
    let Some(reporter) = activity_reporter else {
        return Ok(());
    };
    if slot_count == 0 {
        return Ok(());
    }
    let steps = step_count.clamp(1, 100);
    let step_ms = duration_ms / steps;
    let addressable_slots = slot_count.min(u8::MAX as usize);
    for step in 1..=steps {
        sleep_with_cancellation(step_ms, cancellation).await?;
        let progress = (step as f32) / (steps as f32) * 0.9;
        let remaining_ms = duration_ms.saturating_sub(step * step_ms);
        for index in 0..addressable_slots {
            reporter
                .report_activity(MediaGenerationActivityUpdate {
                    state: ControllerActivityState::Working,
                    event: None,
                    message: String::new(),
                    provider_job_id: None,
                    error_code: None,
                    slot_id: None,
                    slot_index: Some(index as u8),
                    progress: Some(progress),
                    eta_ms: Some(remaining_ms),
                    preview_asset: None,
                    partial_index: None,
                })
                .await?;
        }
    }
    Ok(())
}

fn build_mock_asset(
    generator: &MockMediaGenerator,
    request: &MediaRenderRequest,
    frame: Option<&serde_json::Value>,
    index: usize,
) -> MediaAsset {
    let url = frame
        .and_then(|value| value.get("url"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if matches!(request.kind, MediaKind::Video) {
                read_string_option(
                    &generator.backend.options.extra,
                    "mockUrl",
                    "https://example.invalid/mock/video.mp4",
                )
            } else {
                generator.configured_url.clone().unwrap_or_else(|| {
                    build_prompt_preview_data_url(
                        generator
                            .prompt_preview_font
                            .as_ref()
                            .expect("mock image prompt preview font"),
                        &request.prompt_text,
                    )
                })
            }
        });

    MediaAsset {
        url,
        mime_type: Some(
            frame
                .and_then(|value| value.get("mimeType"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(if matches!(request.kind, MediaKind::Video) {
                    "video/mp4"
                } else {
                    "image/png"
                })
                .to_string(),
        ),
        media_type: if matches!(request.kind, MediaKind::Video) {
            MediaRenderType::Video
        } else {
            MediaRenderType::Image
        },

        width: frame
            .and_then(|value| value.get("width"))
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as u32)
            .or(Some(if matches!(request.kind, MediaKind::Video) {
                1280
            } else {
                2048
            })),
        height: frame
            .and_then(|value| value.get("height"))
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as u32)
            .or(Some(if matches!(request.kind, MediaKind::Video) {
                720
            } else {
                2048
            })),
        duration_seconds: if matches!(request.kind, MediaKind::Video) {
            Some(request.options.duration_seconds.unwrap_or(5))
        } else {
            None
        },

        provider_asset_id: Some(format!("mock-asset-{}", index)),
    }
}

fn load_mock_prompt_font(
    backend: &MediaBackendConfig,
) -> Result<FontArc, EngineAdapterRequestError> {
    let font_path = backend
        .options
        .extra
        .get("fontPath")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            EngineAdapterRequestError::new(
                "mock",
                format!(
                    "Mock image backend \"{}\" requires options.fontPath when options.mockUrl is not set.",
                    backend.id
                ),
                "invalid_request",
            )
        })?;

    let bytes = std::fs::read(font_path).map_err(|error| {
        EngineAdapterRequestError::transport(
            "mock",
            format!(
                "Mock image backend \"{}\" could not read font at \"{}\": {}",
                backend.id, font_path, error
            ),
        )
    })?;

    FontArc::try_from_vec(bytes).map_err(|error| {
        EngineAdapterRequestError::invalid_response(
            "mock",
            format!(
                "Mock image backend \"{}\" could not load font at \"{}\": {}",
                backend.id, font_path, error
            ),
        )
    })
}

fn build_prompt_preview_data_url(font: &FontArc, prompt_text: &str) -> String {
    let width = 2048usize;
    let height = 2048usize;
    let mut pixels = vec![0u8; width * height * 3];
    fill_rect(&mut pixels, width, 0, 0, width, height, [246, 241, 232]);
    fill_rect(&mut pixels, width, 0, 0, width, 216, [34, 94, 78]);
    fill_rect(
        &mut pixels,
        width,
        80,
        280,
        width - 160,
        height - 360,
        [255, 252, 247],
    );

    let wrapped = wrap_text(prompt_text.trim(), 110, 60);
    let body_lines = if wrapped.is_empty() {
        vec!["(empty prompt)".to_string()]
    } else {
        wrapped
    };
    let mut surface = PixelSurface {
        pixels: &mut pixels,
        width,
    };
    let header_style = TextStyle {
        scale: PxScale::from(26.0),
        colour: [248, 246, 240],
        line_gap: 8.0,
    };
    let detail_style = TextStyle {
        scale: PxScale::from(13.0),
        colour: [34, 94, 78],
        line_gap: 6.0,
    };
    let body_style = TextStyle {
        scale: PxScale::from(12.0),
        colour: [46, 53, 64],
        line_gap: 6.0,
    };
    let header_lines = ["MOCK IMAGE", "PROMPT PREVIEW"];
    let detail_lines = ["Prompt sent to image backend:"];
    let body_refs = body_lines.iter().map(String::as_str).collect::<Vec<_>>();

    render_text_block(&mut surface, 136.0, 72.0, header_style, font, &header_lines);
    render_text_block(
        &mut surface,
        152.0,
        340.0,
        detail_style,
        font,
        &detail_lines,
    );
    render_text_block(&mut surface, 152.0, 448.0, body_style, font, &body_refs);

    let mut png_bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut png_bytes, width as u32, height as u32);
        encoder.set_color(ColorType::Rgb);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .expect("mock prompt preview png header");
        writer
            .write_image_data(&pixels)
            .expect("mock prompt preview png body");
    }

    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png_bytes)
    )
}

fn wrap_text(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            lines.push(String::new());
            if lines.len() >= max_lines {
                break;
            }
            continue;
        }

        let mut current = String::new();
        for word in trimmed.split_whitespace() {
            let separator = if current.is_empty() { 0 } else { 1 };
            if current.chars().count() + separator + word.chars().count() <= max_chars {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
                continue;
            }

            if !current.is_empty() {
                lines.push(current);
                if lines.len() >= max_lines {
                    return truncate_last_line(lines, max_chars);
                }
                current = String::new();
            }

            if word.chars().count() <= max_chars {
                current.push_str(word);
                continue;
            }

            let mut chunk = String::new();
            for ch in word.chars() {
                chunk.push(ch);
                if chunk.chars().count() == max_chars {
                    lines.push(chunk);
                    if lines.len() >= max_lines {
                        return truncate_last_line(lines, max_chars);
                    }
                    chunk = String::new();
                }
            }
            current = chunk;
        }

        if !current.is_empty() {
            lines.push(current);
            if lines.len() >= max_lines {
                break;
            }
        }
    }

    if lines.len() > max_lines {
        lines.truncate(max_lines);
    }
    if text.chars().count() > lines.join(" ").chars().count() && !lines.is_empty() {
        return truncate_last_line(lines, max_chars);
    }
    lines
}

fn truncate_last_line(mut lines: Vec<String>, max_chars: usize) -> Vec<String> {
    if let Some(last) = lines.last_mut() {
        let mut truncated = last
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        truncated.push('…');
        *last = truncated;
    }
    lines
}

struct PixelSurface<'a> {
    pixels: &'a mut [u8],
    width: usize,
}

impl PixelSurface<'_> {
    fn height(&self) -> usize {
        self.pixels.len() / (self.width * 3)
    }
}

#[derive(Clone, Copy)]
struct TextStyle {
    scale: PxScale,
    colour: [u8; 3],
    line_gap: f32,
}

fn render_text_block(
    surface: &mut PixelSurface<'_>,
    x: f32,
    mut y: f32,
    style: TextStyle,
    font: &FontArc,
    lines: &[&str],
) {
    let scaled_font = font.as_scaled(style.scale);
    let ascent = scaled_font.ascent();
    let line_height = scaled_font.height() + style.line_gap;

    for line in lines {
        render_text_line(surface, x, y + ascent, style, font, line);
        y += line_height;
    }
}

fn render_text_line(
    surface: &mut PixelSurface<'_>,
    mut x: f32,
    baseline_y: f32,
    style: TextStyle,
    font: &FontArc,
    text: &str,
) {
    let scaled_font = font.as_scaled(style.scale);
    let height = surface.height();

    for ch in text.chars() {
        let glyph_id = font.glyph_id(ch);
        let glyph = glyph_id.with_scale_and_position(style.scale, point(x, baseline_y));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x.floor() as i32 + gx as i32;
                let py = bounds.min.y.floor() as i32 + gy as i32;
                if px < 0 || py < 0 || px as usize >= surface.width || py as usize >= height {
                    return;
                }

                blend_pixel(
                    surface.pixels,
                    surface.width,
                    px as usize,
                    py as usize,
                    style.colour,
                    coverage,
                );
            });
        }
        x += scaled_font.h_advance(glyph_id);
    }
}

fn blend_pixel(
    pixels: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    colour: [u8; 3],
    coverage: f32,
) {
    let offset = (y * width + x) * 3;
    let inverse = 1.0 - coverage;
    pixels[offset] = (pixels[offset] as f32 * inverse + colour[0] as f32 * coverage).round() as u8;
    pixels[offset + 1] =
        (pixels[offset + 1] as f32 * inverse + colour[1] as f32 * coverage).round() as u8;
    pixels[offset + 2] =
        (pixels[offset + 2] as f32 * inverse + colour[2] as f32 * coverage).round() as u8;
}

fn fill_rect(
    pixels: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    rect_width: usize,
    rect_height: usize,
    colour: [u8; 3],
) {
    let height = pixels.len() / (width * 3);
    let max_x = x.saturating_add(rect_width).min(width);
    let max_y = y.saturating_add(rect_height).min(height);
    for py in y..max_y {
        for px in x..max_x {
            let offset = (py * width + px) * 3;
            pixels[offset..offset + 3].copy_from_slice(&colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_prompt_preview_data_url, create_mock_media_generator, read_mock_progress_duration_ms,
        read_mock_progress_step_count,
    };
    use crate::adapter::{EngineAuthConfig, EngineBackendOptions};
    use crate::media::{
        MediaBackendConfig, MediaGenerationActivityReporter, MediaGenerationActivityUpdate,
        MediaRenderRequest,
    };
    use ab_glyph::FontArc;
    use base64::Engine as _;
    use battersea_model::media::{
        MediaBackendCapabilities, MediaCapability, MediaGenerationHints, MediaKind,
        MediaProvenanceTimingSource,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    #[derive(Default)]
    struct RecordingActivityReporter {
        updates: Mutex<Vec<MediaGenerationActivityUpdate>>,
    }

    #[async_trait::async_trait]
    impl MediaGenerationActivityReporter for RecordingActivityReporter {
        async fn report_activity(
            &self,
            update: MediaGenerationActivityUpdate,
        ) -> Result<(), crate::EngineAdapterRequestError> {
            self.updates.lock().expect("updates lock").push(update);
            Ok(())
        }
    }

    fn test_font() -> FontArc {
        for path in [
            "/Library/Fonts/SF-Compact-Text-Regular.otf",
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation2/LiberationMono-Regular.ttf",
            "C:\\Windows\\Fonts\\consola.ttf",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(font) = FontArc::try_from_vec(bytes) {
                    return font;
                }
            }
        }

        panic!("no test font found for mock image renderer");
    }

    fn mock_image_backend(extra: BTreeMap<String, serde_json::Value>) -> MediaBackendConfig {
        MediaBackendConfig {
            id: "mock-image".to_string(),
            provider: "mock".to_string(),
            capability: MediaCapability::ImageGeneration,
            label: "Mock image".to_string(),
            enabled: true,
            endpoint: String::new(),
            model: "mock-image-model".to_string(),
            capabilities: MediaBackendCapabilities::mock(),
            options: EngineBackendOptions {
                extra,
                ..EngineBackendOptions::default()
            },
            auth: EngineAuthConfig {
                auth_type: String::new(),
                api_key_env: String::new(),
                header: None,
                version_header: None,
                version: None,
                has_api_key: false,
                api_key: None,
            },
            short_description: String::new(),
            long_description: String::new(),
        }
    }

    fn render_request(count: Option<u8>) -> MediaRenderRequest {
        MediaRenderRequest {
            kind: MediaKind::Image,
            prompt_text: "x".to_string(),
            negative_prompt: None,
            references: Vec::new(),
            options: MediaGenerationHints {
                count,
                ..MediaGenerationHints::default()
            },
        }
    }

    #[test]
    fn prompt_preview_png_data_url_has_png_signature() {
        let font = test_font();
        let data_url = build_prompt_preview_data_url(&font, "Aiko in cinematic lantern light");
        let encoded = data_url
            .strip_prefix("data:image/png;base64,")
            .expect("png data url");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("decode png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn prompt_preview_png_changes_with_prompt_text() {
        let font = test_font();
        let first = build_prompt_preview_data_url(&font, "Aiko at sunrise");
        let second = build_prompt_preview_data_url(&font, "Aiko at midnight");
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn mock_image_generator_respects_requested_count() {
        let generator = create_mock_media_generator(mock_image_backend(BTreeMap::from([(
            "mockUrl".to_string(),
            json!("data:image/png;base64,AA=="),
        )])))
        .expect("mock generator");

        let rendered = generator
            .generate(
                render_request(Some(3)),
                CancellationToken::new(),
                None,
                None,
            )
            .await
            .expect("mock render");

        assert_eq!(rendered.assets.len(), 3);
        assert_eq!(
            rendered
                .assets
                .iter()
                .filter_map(|asset| asset.provider_asset_id.as_deref())
                .collect::<Vec<_>>(),
            vec!["mock-asset-0", "mock-asset-1", "mock-asset-2"]
        );
    }

    #[tokio::test]
    async fn mock_image_sequence_is_truncated_to_requested_count() {
        let generator = create_mock_media_generator(mock_image_backend(BTreeMap::from([
            (
                "mockSequence".to_string(),
                json!([
                    {"url": "data:image/png;base64,AA=="},
                    {"url": "data:image/png;base64,AA=="},
                    {"url": "data:image/png;base64,AA=="}
                ]),
            ),
            ("mockUrl".to_string(), json!("data:image/png;base64,AA==")),
        ])))
        .expect("mock generator");

        let rendered = generator
            .generate(
                render_request(Some(2)),
                CancellationToken::new(),
                None,
                None,
            )
            .await
            .expect("mock render");

        assert_eq!(rendered.assets.len(), 2);
    }

    #[tokio::test]
    async fn mock_render_emits_deterministic_provider_envelopes() {
        let generator = create_mock_media_generator(mock_image_backend(BTreeMap::from([(
            "mockUrl".to_string(),
            json!("data:image/png;base64,AA=="),
        )])))
        .expect("mock generator");

        let mut request = render_request(Some(1));
        request.prompt_text = "an owl in a library".to_string();
        request.negative_prompt = Some("blurry".to_string());

        let rendered = generator
            .generate(request, CancellationToken::new(), None, None)
            .await
            .expect("mock render");

        let provider_request = rendered
            .provider_request
            .as_ref()
            .expect("mock render captures a request envelope");
        assert_eq!(provider_request["backend"], json!("mock"));
        assert_eq!(provider_request["kind"], json!("image"));
        assert_eq!(
            provider_request["prompt_text"],
            json!("an owl in a library")
        );
        assert_eq!(provider_request["negative_prompt"], json!("blurry"));
        assert_eq!(
            provider_request["mockUrl"],
            json!("data:image/png;base64,AA==")
        );

        let provider_response = rendered
            .provider_response
            .as_ref()
            .expect("mock render captures a response envelope");
        assert_eq!(provider_response["provider_job_id"], json!("mock-job"));
        assert_eq!(provider_response["asset_count"], json!(1));
        assert_eq!(
            provider_response["assets"]
                .as_array()
                .map(|assets| assets.len()),
            Some(1)
        );
    }

    #[test]
    fn mock_progress_duration_defaults_to_the_mock_delay() {
        let options = BTreeMap::from([("mockDelayMs".to_string(), json!(3000))]);
        assert_eq!(read_mock_progress_duration_ms(&options, 3000), 3000);

        let explicit = BTreeMap::from([("mockProgressDurationMs".to_string(), json!(750))]);
        assert_eq!(read_mock_progress_duration_ms(&explicit, 3000), 750);
    }

    #[test]
    fn mock_progress_step_count_defaults_and_clamps() {
        assert_eq!(read_mock_progress_step_count(&BTreeMap::new()), 3);

        let explicit = BTreeMap::from([("mockProgressStepCount".to_string(), json!(10))]);
        assert_eq!(read_mock_progress_step_count(&explicit), 10);

        let too_low = BTreeMap::from([("mockProgressStepCount".to_string(), json!(0))]);
        assert_eq!(read_mock_progress_step_count(&too_low), 1);

        let too_high = BTreeMap::from([("mockProgressStepCount".to_string(), json!(1000))]);
        assert_eq!(read_mock_progress_step_count(&too_high), 100);
    }

    #[tokio::test]
    async fn mock_render_emits_provider_progress_for_each_requested_slot() {
        let generator = create_mock_media_generator(mock_image_backend(BTreeMap::from([
            ("mockUrl".to_string(), json!("data:image/png;base64,AA==")),
            ("mockDelayMs".to_string(), json!(0)),
        ])))
        .expect("mock generator");
        let reporter = Arc::new(RecordingActivityReporter::default());

        let rendered = generator
            .generate(
                render_request(Some(2)),
                CancellationToken::new(),
                Some(reporter.clone()),
                None,
            )
            .await
            .expect("mock render");

        assert_eq!(rendered.assets.len(), 2);
        let updates = reporter.updates.lock().expect("updates lock");
        let progress_updates = updates
            .iter()
            .filter(|update| update.progress.is_some())
            .collect::<Vec<_>>();
        assert_eq!(progress_updates.len(), 6);
        assert_eq!(
            progress_updates
                .iter()
                .filter(|update| update.slot_index == Some(0))
                .filter_map(|update| update.progress)
                .collect::<Vec<_>>(),
            vec![0.3_f32, 0.6_f32, 0.9_f32]
        );
        assert_eq!(
            progress_updates
                .iter()
                .filter(|update| update.slot_index == Some(1))
                .filter_map(|update| update.progress)
                .collect::<Vec<_>>(),
            vec![0.3_f32, 0.6_f32, 0.9_f32]
        );
    }

    #[tokio::test]
    async fn mock_render_honours_configured_progress_step_count() {
        let generator = create_mock_media_generator(mock_image_backend(BTreeMap::from([
            ("mockUrl".to_string(), json!("data:image/png;base64,AA==")),
            ("mockDelayMs".to_string(), json!(0)),
            ("mockProgressStepCount".to_string(), json!(5)),
        ])))
        .expect("mock generator");
        let reporter = Arc::new(RecordingActivityReporter::default());

        generator
            .generate(
                render_request(Some(1)),
                CancellationToken::new(),
                Some(reporter.clone()),
                None,
            )
            .await
            .expect("mock render");

        let updates = reporter.updates.lock().expect("updates lock");
        let progress_values = updates
            .iter()
            .filter_map(|update| update.progress)
            .collect::<Vec<_>>();
        assert_eq!(progress_values.len(), 5);
        assert!(
            (progress_values[0] - 0.18).abs() < f32::EPSILON,
            "first tick should be 18%, got {:?}",
            progress_values[0]
        );
        assert_eq!(progress_values[4], 0.9_f32);
    }

    #[tokio::test]
    async fn mock_render_with_delay_records_client_estimate_timing() {
        let generator = create_mock_media_generator(mock_image_backend(BTreeMap::from([
            ("mockUrl".to_string(), json!("data:image/png;base64,AA==")),
            ("mockDelayMs".to_string(), json!(5)),
        ])))
        .expect("mock generator");

        let rendered = generator
            .generate(
                render_request(Some(1)),
                CancellationToken::new(),
                None,
                None,
            )
            .await
            .expect("mock render");

        let timing = rendered.timing.expect("timing");
        assert!(timing.elapsed_ms >= 1);
        assert!(timing.model_processing_ms.unwrap_or_default() >= 1);
        assert_eq!(
            timing.model_processing_source,
            MediaProvenanceTimingSource::ClientEstimate
        );
        assert!(timing
            .phases
            .iter()
            .any(|phase| phase.included_in_model_processing));
    }
}
