//! Opt-in native application workflow evidence, using the ordinary workers and
//! Save handler. It does not inject OS input or claim to test native dialogs.
use super::*;

impl Editor {
    fn quality_capture_step(
        &mut self,
        ctx: &egui::Context,
        stage: u8,
        path: &std::path::Path,
    ) -> Result<u8, String> {
        match stage {
            0 => {
                let options = std::env::var_os("EMBROIDERYLAB_CAPTURE_OPTIONS")
                    .ok_or("Quality capture requires explicit options")?;
                self.raster_options =
                    serde_json::from_slice(&std::fs::read(options).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                self.import_width_mm = std::env::var("EMBROIDERYLAB_CAPTURE_WIDTH_MM")
                    .map_err(|e| e.to_string())?
                    .parse::<f64>()
                    .map_err(|e| e.to_string())?;
                // The ordinary modal binds its source mask before the Preview
                // button is available. Freeze the same state here, otherwise
                // the next frame correctly invalidates this probe's job.
                let pixels = self
                    .import_preview
                    .as_ref()
                    .ok_or("Quality capture requires an image preview")?
                    .pixels
                    .clone();
                self.cleanup
                    .reset(&pixels, self.raster_options.cleanup.as_ref());
                self.raster_options.cleanup = Some(self.cleanup.mask(&pixels)?);
                self.preview_regions(ctx);
                Ok(1)
            }
            1 if self.digitize_job.is_none() => {
                if self.error || self.region_preview.is_none() || self.import_trace_report.is_none()
                {
                    return Err(format!("Native region preview failed: {}", self.message));
                }
                self.cleanup.view = 4;
                // Paint the changed-pixel layer for a complete frame first.
                Ok(2)
            }
            2 => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                Ok(3)
            }
            3 => {
                let screenshot = ctx.input(|i| {
                    i.events.iter().find_map(|event| match event {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(screenshot) = screenshot {
                    let rgba: Vec<u8> = screenshot
                        .pixels
                        .iter()
                        .flat_map(|p| p.to_array())
                        .collect();
                    image::save_buffer(
                        path.with_extension("changes.png"),
                        &rgba,
                        screenshot.width() as u32,
                        screenshot.height() as u32,
                        image::ColorType::Rgba8,
                    )
                    .map_err(|e| e.to_string())?;
                    std::fs::write(
                        path.with_extension("trace.json"),
                        serde_json::to_vec_pretty(self.import_trace_report.as_ref().unwrap())
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    self.commit_import(ctx);
                    Ok(4)
                } else {
                    Ok(3)
                }
            }
            4 if self.digitize_job.is_none() => {
                if self.error
                    || self.stale
                    || self.import_preview.is_some()
                    || self.stitches.is_empty()
                {
                    return Err(format!("Native import/generation failed: {}", self.message));
                }
                self.path = Some(path.with_extension("elab"));
                self.save();
                if self.error || self.dirty {
                    return Err(format!("Native Save failed: {}", self.message));
                }
                let reopened = Project::load(self.path.as_ref().unwrap())?;
                let project_equal = serde_json::to_value(&reopened).map_err(|e| e.to_string())?
                    == serde_json::to_value(&self.project).map_err(|e| e.to_string())?;
                // The export dialog is outside this programmatic probe. Use
                // the same public codec, preserving the applied machine plan.
                let dst_bytes = dst::encode_with_trim(
                    &self.stitches,
                    &self.project.name,
                    self.project.machine_profile.dst_trim_encoding,
                )?;
                let decoded = dst::decode(&dst_bytes)?;
                std::fs::write(path.with_extension("dst"), dst_bytes).map_err(|e| e.to_string())?;
                let plan = MachinePlan {
                    stitches: self.stitches.clone(),
                    metadata: self.plan_metadata.clone(),
                    warnings: self.plan_warnings.clone(),
                };
                std::fs::write(
                    path.with_extension("plan.json"),
                    serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                std::fs::write(
                    path.with_extension("threads.json"),
                    serde_json::to_vec_pretty(&embroidery_lab::commands::thread_sidecar(
                        &self.project,
                        &self.stitches,
                    ))
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let result = serde_json::json!({
                    "kind": "native-application-programmatic-workflow", "passed": project_equal,
                    "real_egui_opengl_window": true, "region_preview_worker": true,
                    "changed_pixel_frame_saved": true, "import_generation_worker": true,
                    "ordinary_save_handler": true, "reopened_project_equal": project_equal,
                    "export_codec_and_decode": true, "decoded_commands": decoded.len(),
                    "native_os_input": "NOT VERIFIED: Computer Use list_windows timed out",
                    "native_export_dialog": "NOT VERIFIED: codec invoked programmatically",
                    "physical_sewout": false,
                });
                std::fs::write(
                    path.with_extension("workflow.json"),
                    serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                self.show_stitches = true;
                self.fit = true;
                Ok(5)
            }
            _ => Ok(stage),
        }
    }

    pub(super) fn capture_quality_workflow(&mut self, ctx: &egui::Context) -> bool {
        if std::env::var("EMBROIDERYLAB_CAPTURE_QUALITY_WORKFLOW").as_deref() != Ok("1") {
            return false;
        }
        let Some(path) = self.capture_path.clone() else {
            return false;
        };
        let id = egui::Id::new("quality-capture-stage");
        let stage = ctx.data(|d| d.get_temp::<u8>(id).unwrap_or(0));
        if stage == 5 {
            return false;
        }
        let next = match self.quality_capture_step(ctx, stage, &path) {
            Ok(next) => next,
            Err(error) => {
                let _ = std::fs::write(
                    path.with_extension("workflow.json"),
                    serde_json::json!({"passed":false,"error":error,"stage":stage}).to_string(),
                );
                self.result(Err(error));
                self.cancel_digitize();
                5
            }
        };
        ctx.data_mut(|d| d.insert_temp(id, next));
        ctx.request_repaint();
        next != 5
    }
}
