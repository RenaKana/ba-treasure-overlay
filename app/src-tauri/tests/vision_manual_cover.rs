#[path = "../src/vision.rs"]
mod vision;

use image::{imageops, Rgba, RgbaImage};
use vision::{cover_reference::CoverSample, Recognizer};

fn initial() -> RgbaImage {
    let frame = image::load_from_memory(include_bytes!("fixtures/vision-initial.png"))
        .unwrap()
        .to_rgba8();
    imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image()
}

// An unfamiliar borderless appearance that the automatic closed-frame
// bootstrap cannot confirm. The player can explicitly label it instead.
fn manual_theme() -> RgbaImage {
    let mut frame = initial();
    for y in 286..806 {
        for x in 908..1844 {
            let column = (x - 908) / 104;
            frame.put_pixel(
                x,
                y,
                Rgba(if column % 2 == 0 {
                    [30, 15, 160, 255]
                } else {
                    [160, 25, 40, 255]
                }),
            );
        }
    }
    frame
}

fn samples(frame: &RgbaImage) -> Vec<CoverSample> {
    let all = vision::sample_cover_candidates(frame, None, None).expect("preview proposals");
    assert_eq!(all.len(), 45);
    vec![all[0].clone(), all[1].clone()]
}

fn real_samples(frame: &RgbaImage) -> Vec<CoverSample> {
    let all = vision::sample_cover_candidates(frame, None, None).expect("real preview proposals");
    all[..5].to_vec()
}

#[test]
fn five_real_appearances_match_all_positions_and_resized_rasters() {
    let frame = initial();
    let references = real_samples(&frame);
    for width in [960, 1440, 1920, 2560] {
        let scaled = imageops::resize(&frame, width, width * 9 / 16, imageops::FilterType::Triangle);
        let mut recognizer = Recognizer::with_cover_samples(&references);
        let result = recognizer.analyze_completed_snapshot(&scaled, None, Some(45), [None; 3]);
        assert!(result.present, "width={width}: {}", result.message);
        assert!(result.cells.iter().all(|c| c == "unknown"), "width={width}: {:?}", result.cells);
        assert!(!result.manual_cover_mismatch);
    }
}

#[test]
fn five_real_appearances_match_a_picturebox_zoom_and_dpi_canvas() {
    let source = image::load_from_memory(include_bytes!("fixtures/vision-initial.png")).unwrap().to_rgba8();
    // PictureBox.Zoom rounds its fitted raster to 1213x720, then a 150% DPI
    // compositor stretches the logical client. Retain the captured title bar
    // and measured 16:9 content transform from that native replay.
    let zoomed = imageops::resize(&source, 1213, 720, imageops::FilterType::Triangle);
    let mut logical = RgbaImage::from_pixel(1280, 720, Rgba([240, 240, 240, 255]));
    imageops::overlay(&mut logical, &zoomed, 33, 0);
    let physical = imageops::resize(&logical, 1920, 1080, imageops::FilterType::Triangle);
    let mut frame = RgbaImage::from_pixel(1922, 1128, Rgba([240, 240, 240, 255]));
    imageops::overlay(&mut frame, &physical, 1, 47);
    let content = [50.0, 103.25, 1820.0, 1023.75];
    let all = vision::sample_cover_candidates(&frame, Some(content), None).unwrap();
    let encoded = serde_json::to_vec(&all[..5]).unwrap();
    let references: Vec<CoverSample> = serde_json::from_slice(&encoded).unwrap();
    let mut recognizer = Recognizer::with_cover_samples(&references);
    let result = recognizer.analyze_completed_in_content(&frame, content, None, Some(45), [None; 3]);
    assert!(result.present, "{}", result.message);
    assert!(result.cells.iter().all(|c| c == "unknown"), "{:?}", result.cells);
    assert!(!result.manual_cover_mismatch);
}

#[test]
fn an_unselected_real_appearance_only_hints_to_update_samples() {
    let frame = initial();
    let references = real_samples(&frame);
    let mut recognizer = Recognizer::with_cover_samples(&references[..1]);
    for _ in 0..3 {
        let result = recognizer.analyze(&frame, None, Some(45));
        assert!(result.present, "{}", result.message);
        assert!(result.cells.iter().any(|c| c == "uncertain"));
        assert!(result.manual_cover_mismatch);
        assert_eq!(result.message, "未翻开样本不匹配，请检查画面或更新样本");
    }
}

#[test]
fn player_samples_locate_an_unfamiliar_board_and_match_at_any_position() {
    let frame = manual_theme();
    assert!(Recognizer::new().locate_content(&frame).is_none());
    let references = samples(&frame);
    let mut recognizer = Recognizer::with_cover_samples(&references);
    let result = recognizer.analyze_completed_snapshot(&frame, None, Some(45), [None; 3]);
    assert!(result.present, "{}", result.message);
    assert!(result.cells.iter().all(|c| c == "unknown"));
    recognizer.reset();
    let result = recognizer.analyze(&frame, None, Some(45));
    assert!(result.present, "fixed references survive a round reset");
    assert!(result.cells.iter().all(|c| c == "unknown"));
}

#[test]
fn mismatch_never_relearns_or_falls_back_to_a_different_automatic_cover() {
    let frame = manual_theme();
    let references = samples(&frame);
    let mut recognizer = Recognizer::with_cover_samples(&references[..1]);
    for _ in 0..3 {
        let result = recognizer.analyze(&frame, None, Some(45));
        assert!(
            result.cells.iter().any(|c| c != "unknown"),
            "unselected appearance cannot be learned"
        );
    }
    let original = initial();
    for _ in 0..3 {
        let result = recognizer.analyze(&original, None, Some(45));
        assert!(
            !result.present,
            "embedded cover matches must not override fixed samples"
        );
    }
    assert!(
        Recognizer::new().analyze(&original, None, Some(45)).present,
        "clearing manual samples restores the automatic path"
    );
}

#[test]
fn small_new_fragment_remains_changed_after_manual_reference_selection() {
    let mut frame = manual_theme();
    let mut recognizer = Recognizer::with_cover_samples(&samples(&frame));
    for y in 333..343 {
        for x in 955..965 {
            frame.put_pixel(x, y, Rgba([0, 240, 20, 255]));
        }
    }
    let result = recognizer.analyze(&frame, None, Some(44));
    assert_ne!(result.cells[0], "unknown");
    assert_eq!(result.cells.iter().filter(|c| *c == "unknown").count(), 44);
    assert!(!result.manual_cover_mismatch, "partial artwork is not a closed-board mismatch");
}

#[test]
fn small_new_fragment_near_the_cover_edge_remains_changed() {
    let mut frame = manual_theme();
    let mut recognizer = Recognizer::with_cover_samples(&samples(&frame));
    // Normalized x=2..5 is inside the manual comparison, beside the gutter.
    for y in 333..343 {
        for x in 915..925 {
            frame.put_pixel(x, y, Rgba([0, 240, 20, 255]));
        }
    }
    let result = recognizer.analyze(&frame, None, Some(44));
    assert_ne!(result.cells[0], "unknown");
    assert_eq!(result.cells.iter().filter(|c| *c == "unknown").count(), 44);
    assert!(!result.manual_cover_mismatch);
}

#[test]
fn manual_samples_survive_window_resampling_and_serialization() {
    let frame = manual_theme();
    let encoded = serde_json::to_vec(&samples(&frame)).unwrap();
    let restored: Vec<CoverSample> = serde_json::from_slice(&encoded).unwrap();
    let mut recognizer = Recognizer::with_cover_samples(&restored);
    for width in [960, 1440, 1920] {
        let scaled = imageops::resize(
            &frame,
            width,
            width * 9 / 16,
            imageops::FilterType::Triangle,
        );
        let result = recognizer.analyze(&scaled, None, Some(45));
        assert!(result.present, "width={width}: {}", result.message);
        assert!(result.cells.iter().all(|c| c == "unknown"));
    }
}

#[test]
fn sampling_a_blank_or_unrelated_frame_never_provides_candidates() {
    assert!(vision::sample_cover_candidates(&RgbaImage::new(1920, 1080), None, None).is_none());
}
