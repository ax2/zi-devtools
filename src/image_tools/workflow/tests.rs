use super::*;
fn source() -> Arc<DynamicImage> {
    Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(
        20,
        16,
        |x, y| {
            image::Rgba([
                x as u8 * 10,
                y as u8 * 10,
                90,
                if x % 3 == 0 { 0 } else { 180 },
            ])
        },
    )))
}
#[test]
fn sequential_crop_coordinates_and_real_webp_report_keep_source() {
    let source = source();
    let original = source.to_rgba8();
    let def = Definition {
        steps: vec![
            Step::Crop {
                version: 1,
                x: 2500,
                y: 0,
                width: 5000,
                height: 10000,
            },
            Step::Crop {
                version: 1,
                x: 5000,
                y: 0,
                width: 5000,
                height: 10000,
            },
            Step::Encode {
                version: 1,
                format: Encoding::Webp,
                quality: 80,
            },
            Step::Info { version: 1 },
        ],
        ..Definition::default()
    };
    let run = execute(&def, source.clone(), &AtomicBool::new(false)).unwrap();
    assert!(!run.cancelled && run.failure.is_none());
    assert_eq!(run.outputs.len(), 4);
    let expected = source.crop_imm(10, 0, 5, 16).to_rgba8();
    for out in &run.outputs[1..] {
        assert_eq!(out.image.to_rgba8(), expected);
    }
    let last = run.outputs.last().unwrap();
    let bytes = last.encoded.as_ref().unwrap();
    assert_eq!(image::load_from_memory(bytes).unwrap().to_rgba8(), expected);
    let report: serde_json::Value = serde_json::from_str(last.report.as_ref().unwrap()).unwrap();
    assert_eq!(report["encoding"]["format"], "webp");
    assert_eq!(report["encoding"]["bytes"], bytes.len());
    assert_eq!(report["width"], 5);
    assert_eq!(source.to_rgba8(), original);
    assert!(Arc::ptr_eq(&run.outputs[2].image, &run.outputs[3].image));
}
#[test]
fn encoders_use_actual_decoded_pixels_and_resize_never_upscales() {
    for format in [Encoding::Png, Encoding::Jpeg, Encoding::Webp] {
        let def = Definition {
            steps: vec![
                Step::Resize {
                    version: 1,
                    max_width: 100,
                },
                Step::Resize {
                    version: 1,
                    max_width: 10,
                },
                Step::Encode {
                    version: 1,
                    format,
                    quality: 75,
                },
            ],
            ..Definition::default()
        };
        let src = source();
        let run = execute(&def, src.clone(), &AtomicBool::new(false)).unwrap();
        assert!(run.failure.is_none());
        assert!(Arc::ptr_eq(&run.outputs[0].image, &src));
        let out = &run.outputs[2];
        assert_eq!(out.image.dimensions(), (10, 8));
        let bytes = out.encoded.as_ref().unwrap();
        assert_eq!(
            image::guess_format(bytes).unwrap(),
            format.native().image_format()
        );
        assert_eq!(
            out.image.to_rgba8(),
            image::load_from_memory(bytes).unwrap().to_rgba8()
        );
    }
}
#[test]
fn definition_roundtrip_rejects_authority_unknown_versions_and_invalid_geometry() {
    let def = Definition::default();
    let bytes = def.bytes().unwrap();
    assert_eq!(Definition::parse(&bytes).unwrap(), def);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["path"] = serde_json::json!("C:/hidden.png");
    assert!(Definition::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    value.as_object_mut().unwrap().remove("path");
    value["steps"][0]["version"] = serde_json::json!(2);
    assert!(Definition::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(Definition::parse(&vec![b' '; RECIPE_LIMIT + 1]).is_err());
    for step in [
        Step::Crop {
            version: 1,
            x: u32::MAX,
            y: 0,
            width: 1,
            height: 10000,
        },
        Step::Crop {
            version: 1,
            x: 0,
            y: 0,
            width: 0,
            height: 10000,
        },
        Step::Resize {
            version: 1,
            max_width: 0,
        },
        Step::Encode {
            version: 1,
            format: Encoding::Png,
            quality: 1,
        },
    ] {
        assert!(
            Definition {
                steps: vec![step],
                ..def.clone()
            }
            .validate()
            .is_err()
        );
    }
    let root = std::env::temp_dir().join(format!("zi-image-recipe-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let path = root.join("配方.json");
    crate::local_files::save_new_moved(&path, &bytes, &AtomicBool::new(false)).unwrap();
    let m = crate::material_files::FileMaterial::selected(&path, RECIPE_LIMIT).unwrap();
    assert_eq!(
        Definition::parse(&m.read_bytes(RECIPE_LIMIT).unwrap()).unwrap(),
        def
    );
    assert!(
        crate::local_files::save_new_moved(&path, b"replace", &AtomicBool::new(false)).is_err()
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn cumulative_budget_stops_before_allocation_and_preserves_completed_steps() {
    let source = Arc::new(DynamicImage::new_rgba8(4000, 2000));
    let def = Definition {
        steps: vec![
            Step::Crop {
                version: 1,
                x: 0,
                y: 0,
                width: 10000,
                height: 10000
            };
            16
        ],
        ..Definition::default()
    };
    let run = execute(&def, source.clone(), &AtomicBool::new(false)).unwrap();
    assert_eq!(run.outputs.len(), 7);
    assert!(run.failure.as_ref().unwrap().contains("256MiB"));
    assert!(!run.cancelled);
    assert_eq!(source.dimensions(), (4000, 2000));
    let cancelled = execute(&def, source, &AtomicBool::new(true)).unwrap();
    assert!(cancelled.cancelled && cancelled.outputs.is_empty());
}
#[test]
fn inactive_workflow_receives_result_and_blocks_exit_until_terminal() {
    let ctx = egui::Context::default();
    let (tx, rx) = mpsc::channel();
    let mut images = super::super::State::default();
    images.workflow.receiver = Some(rx);
    assert!(images.background_active());
    tx.send(Ok(Reply::Run(
        execute(&Definition::default(), source(), &AtomicBool::new(false)).unwrap(),
    )))
    .unwrap();
    images.poll_screenshot(&ctx);
    assert!(!images.background_active());
    assert_eq!(images.workflow.run.as_ref().unwrap().outputs.len(), 3);
    assert!(images.workflow.relay_source().is_some());
    images.workflow.invalidate();
    assert!(images.workflow.relay_source().is_none());
}
#[test]
fn input_replacement_preserves_definition_and_import_blocks_exit() {
    let mut state = State::default();
    state.definition.steps = vec![Step::Resize {
        version: 1,
        max_width: 10,
    }];
    let definition = state.definition.clone();
    state.run = Some(execute(&definition, source(), &AtomicBool::new(false)).unwrap());
    let prepared = relay::prepare(relay::Source::Image(source()), vec![], "image-tools").unwrap();
    state.receive(&prepared);
    assert_eq!(state.definition, definition);
    assert!(state.run.is_none());
    assert!(Arc::ptr_eq(state.source.as_ref().unwrap(), &prepared.image));
    assert_eq!(state.origins[0].id, "image-tools");
    let mut images = super::super::State::default();
    images.workflow.import = Some(Definition::default());
    assert!(images.background_active());
    assert_eq!(images.workflow.target_state(), (true, false));
    images.workflow.import = None;
    assert!(!images.background_active());
}
