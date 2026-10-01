#[path = "../src/ocr.rs"]
mod ocr;
#[path = "../src/vision.rs"]
mod vision;

#[test]
fn pc_global_zh_hant_initial() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }

    let frame = image::load_from_memory(include_bytes!("fixtures/pc-global-zh-hant-initial.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(frame.dimensions(), (1284, 767));

    let hud = ocr::read(&frame);
    assert_eq!(hud.remaining, Some(45));
    assert_eq!(hud.round.as_deref(), Some("1"));
    assert_eq!(hud.counts, [Some(2), Some(5), Some(2)]);

    let result = vision::Recognizer::new().analyze_completed_snapshot(
        &frame,
        None,
        hud.remaining,
        hud.counts,
    );
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells.len(), 45);
    assert!(result.cells.iter().all(|cell| cell == "unknown"));
    assert_eq!(result.shapes, vec![[3, 2], [3, 1], [2, 1]]);
    assert_eq!(result.reference_ready, [true; 3]);
    assert!(result.completed_objects.is_empty());
    assert!(result.candidate_constraints.is_empty());
    assert_eq!(result.finish, [false; 3]);

    let [x, y, width, height] = result.board.expect("board should be detected");
    assert!((x * frame.width() as f64 - 607.33).abs() <= 2.0);
    assert!((y * frame.height() as f64 - 235.67).abs() <= 2.0);
    assert!((width * frame.width() as f64 - 624.0).abs() <= 2.0);
    assert!((height * frame.height() as f64 - 346.67).abs() <= 2.0);
}

#[test]
fn pc_global_zh_hant_native_four_three_initial() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }
    let frame = image::load_from_memory(include_bytes!(
        "fixtures/pc-global-zh-hant-four-three-initial.png"
    ))
    .unwrap()
    .to_rgba8();
    assert_eq!(frame.dimensions(), (1284, 1007));
    let locate_started = std::time::Instant::now();
    let content = vision::locate_content(&frame).expect("native 4:3 content");
    eprintln!(
        "native 4:3 content location: {:?}",
        locate_started.elapsed()
    );
    assert_eq!(content, [2.0, 45.0, 1280.0, 960.0]);
    let hud = ocr::read_in_content(&frame, content);
    assert_eq!(hud.remaining, Some(45));
    assert_eq!(hud.round.as_deref(), Some("1"));
    assert_eq!(hud.counts, [Some(2), Some(5), Some(2)]);
    let result = vision::Recognizer::new().analyze_completed_in_content(
        &frame,
        content,
        None,
        hud.remaining,
        hud.counts,
    );
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells, vec!["unknown"; 45]);
    assert_eq!(result.shapes, vec![[3, 2], [3, 1], [2, 1]]);
    assert_eq!(result.reference_ready, [true; 3]);
    assert_eq!(result.finish, [false; 3]);
    assert!(result.completed_objects.is_empty());
    assert!(result.candidate_constraints.is_empty());
    let [x, y, w, h] = result.board.expect("native 4:3 board");
    assert!((x * frame.width() as f64 - 607.3333).abs() < 0.01);
    assert!((y * frame.height() as f64 - 355.6667).abs() < 0.01);
    assert!((w * frame.width() as f64 - 624.0).abs() < 0.01);
    assert!((h * frame.height() as f64 - 346.6667).abs() < 0.01);

    // Existing calibration is stored on the centered 16:9 layout canvas.
    // Reusing it on a taller client must translate, never stretch, the board.
    let saved = [
        908.0 / 1920.0,
        286.0 / 1080.0,
        936.0 / 1920.0,
        520.0 / 1080.0,
    ];
    let [cx, cy, cw, ch] = vision::layout_region(content, vision::LayoutAnchor::Center);
    let manual = [
        (cx + saved[0] * cw) / frame.width() as f64,
        (cy + saved[1] * ch) / frame.height() as f64,
        saved[2] * cw / frame.width() as f64,
        saved[3] * ch / frame.height() as f64,
    ];
    let calibrated = vision::Recognizer::new().analyze_completed_in_content(
        &frame,
        content,
        Some(manual),
        hud.remaining,
        hud.counts,
    );
    assert!(calibrated.present, "{}", calibrated.message);
    for (manual, automatic) in calibrated.board.unwrap().iter().zip(result.board.unwrap()) {
        assert!((manual - automatic).abs() < 1e-12);
    }
    assert_eq!(calibrated.cells, result.cells);
}
