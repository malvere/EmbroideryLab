//! Opt-in capture evidence for a real trace worker and the ordinary cancel handler.
//! No synthetic progress, worker pauses, engine changes or OS-click claims.
use super::*;
use digitize_job::JobOutput;
use std::sync::{atomic::Ordering, mpsc};

type Reply = Result<JobOutput, String>;

#[derive(Clone, Copy)]
enum Step {
    Constructor,
    Prime,
    Trace,
    Screenshot,
    Cancelled,
}

struct Snapshot {
    document: Vec<u8>,
    plan: Vec<u8>,
    stale: bool,
}
impl Snapshot {
    fn read(app: &Editor) -> Result<Self, String> {
        Ok(Self {
            document: serde_json::to_vec_pretty(&app.project).map_err(|e| e.to_string())?,
            plan: serde_json::to_vec_pretty(&MachinePlan {
                stitches: app.stitches.clone(),
                metadata: app.plan_metadata.clone(),
                warnings: app.plan_warnings.clone(),
            })
            .map_err(|e| e.to_string())?,
            stale: app.stale,
        })
    }
}

pub(super) struct WorkerProbe {
    path: PathBuf,
    input: PathBuf,
    step: Step,
    receiver: Option<mpsc::Receiver<Reply>>,
    before: Option<Snapshot>,
    report: serde_json::Value,
    started: Instant,
}

fn detach_receiver(app: &mut Editor) -> Option<mpsc::Receiver<Reply>> {
    let job = app.digitize_job.as_mut()?;
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    Some(std::mem::replace(&mut job.receiver, receiver))
}

impl WorkerProbe {
    // Arm before request_open cancels the constructor job, retaining only its
    // receiver so the first real job is observed to finish before priming.
    pub(super) fn arm(app: &mut Editor) -> Option<Self> {
        if std::env::var("EMBROIDERYLAB_CAPTURE_WORKER_PROBE").as_deref() != Ok("1") {
            return None;
        }
        let path = app.capture_path.clone()?;
        let input = PathBuf::from(std::env::args_os().nth(1)?);
        let extension = input.extension()?.to_str()?.to_ascii_lowercase();
        if !input.is_file() || !["png", "jpg", "jpeg"].contains(&extension.as_str()) {
            return None;
        }
        Some(Self {
            path,
            input: input.clone(),
            step: Step::Constructor,
            receiver: detach_receiver(app),
            before: None,
            started: Instant::now(),
            report: serde_json::json!({
                "kind":"real-worker-capture-probe", "source":input,
                "native_os_cancel":"UNVERIFIED; programmatic application handler only",
                "artificial_worker_pause":false, "preview_only":true,
                "screenshot_requested_while_pending":false, "screenshot_saved":false,
                "cancel_handler_called":false, "cancel_requested_before_completion":false,
                "worker_result_observed":false, "passed":false
            }),
        })
    }

    pub(super) fn matches_preview(&self, app: &Editor) -> bool {
        app.import_preview
            .as_ref()
            .is_some_and(|p| p.pdf.is_none() && p.path == self.input)
    }

    fn write_snapshot(&self, snapshot: &Snapshot, suffix: &str) -> Result<(), String> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(
            self.path.with_extension(format!("{suffix}.elab")),
            &snapshot.document,
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(
            self.path.with_extension(format!("{suffix}.plan.json")),
            &snapshot.plan,
        )
        .map_err(|e| e.to_string())
    }

    fn finish(&mut self, app: &mut Editor, ctx: &egui::Context, failure: Option<&str>) -> bool {
        if let Some(failure) = failure {
            self.report["failure"] = failure.into();
        }
        match Snapshot::read(app).and_then(|after| {
            self.write_snapshot(&after, "after")?;
            Ok(after)
        }) {
            Ok(after) => {
                if let Some(before) = &self.before {
                    self.report["document_bytes_equal"] =
                        (before.document == after.document).into();
                    self.report["plan_bytes_equal"] = (before.plan == after.plan).into();
                    self.report["stale_before"] = before.stale.into();
                    self.report["stale_after"] = after.stale.into();
                    self.report["passed"] = (failure.is_none()
                        && self.report.get("failure").is_none()
                        && self.report["screenshot_saved"] == true
                        && self.report["screenshot_requested_while_pending"] == true
                        && self.report["cancel_handler_called"] == true
                        && self.report["cancel_token_before"] == false
                        && self.report["cancel_token_after"] == true
                        && self.report["worker_result_observed"] == true
                        && self.report["job_id_at_screenshot_request"]
                            == self.report["job_id_at_cancel"]
                        && self.report["cancel_requested_before_completion"] == true
                        && before.document == after.document
                        && before.plan == after.plan
                        && !before.stale
                        && !after.stale)
                        .into();
                }
            }
            Err(error) => self.report["failure"] = error.into(),
        }
        self.report["elapsed_ms"] = (self.started.elapsed().as_millis() as u64).into();
        let written = serde_json::to_vec_pretty(&self.report)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                std::fs::write(self.path.with_extension("probe.json"), bytes)
                    .map_err(|e| e.to_string())
            });
        if let Err(error) = written {
            eprintln!("Worker probe report failed: {error}");
        }
        // Failed probes are still capture sessions: do not persist their
        // temporary workspace geometry during the ordinary on_exit hook.
        app.capture_requested = true;
        app.capture_path = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        true
    }

    pub(super) fn frame(&mut self, app: &mut Editor, ctx: &egui::Context) -> bool {
        match self.step {
            Step::Constructor => {
                if let Some(receiver) = &self.receiver
                    && matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty))
                {
                    ctx.request_repaint_after(std::time::Duration::from_millis(16));
                    return false;
                }
                self.receiver = None;
                self.report["constructor_result_observed_or_disconnected"] = true.into();
                app.generate();
                self.step = Step::Prime;
            }
            Step::Prime => {
                // Normal digitize_progress applies the real demo generation.
                if app.digitize_job.is_some() {
                    ctx.request_repaint();
                    return false;
                }
                if app.stale
                    || app.stitches.is_empty()
                    || app.error
                    || app.plan_metadata.len() != app.stitches.len()
                {
                    return self.finish(
                        app,
                        ctx,
                        Some("Real demo generation did not produce a current valid plan"),
                    );
                }
                match Snapshot::read(app).and_then(|before| {
                    self.write_snapshot(&before, "before")?;
                    Ok(before)
                }) {
                    Ok(before) => self.before = Some(before),
                    Err(error) => return self.finish(app, ctx, Some(&error)),
                }
                self.report["last_valid_commands"] = app.stitches.len().into();
                self.report["last_valid_project_name"] = app.project.name.clone().into();
                if let Some(preview) = &app.import_preview {
                    self.report["source_pixels"] =
                        serde_json::json!([preview.pixels.width(), preview.pixels.height()]);
                }
                app.start_trace_job(true);
                self.step = Step::Trace;
                // Do not capture this frame: the import job's real progress
                // modal will be painted by the normal UI on the next frame.
            }
            Step::Trace | Step::Screenshot => {
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|event| match event {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(image) = &image {
                    self.report["screenshot_pixels"] =
                        serde_json::json!([image.width(), image.height()]);
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    match image::save_buffer(
                        &self.path,
                        &rgba,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => self.report["screenshot_saved"] = true.into(),
                        Err(error) => self.report["failure"] = error.to_string().into(),
                    }
                }
                let Some(job) = &app.digitize_job else {
                    self.report["completion_before_cancel"] = true.into();
                    self.report["worker_result_observed"] = true.into();
                    self.report["completion_observed_by"] = "normal GUI result poll".into();
                    return self.finish(
                        app,
                        ctx,
                        Some("Trace completed before the screenshot/cancel frame"),
                    );
                };
                if !matches!(job.receiver.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                    self.report["completion_before_cancel"] = true.into();
                    self.report["worker_result_observed"] = true.into();
                    self.report["completion_observed_by"] = "probe pre-cancel result check".into();
                    app.cancel_digitize();
                    return self.finish(
                        app,
                        ctx,
                        Some("Trace result was already available before cancellation"),
                    );
                }
                if matches!(self.step, Step::Trace) {
                    self.report["job_id_at_screenshot_request"] = job.id.into();
                    self.report["stage_at_screenshot_request"] =
                        job.stage.load(Ordering::Relaxed).into();
                    self.report["revision_at_screenshot_request"] = app.job_revision.into();
                    self.report["screenshot_requested_while_pending"] = true.into();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                    app.capture_requested = true;
                    self.step = Step::Screenshot;
                } else if image.is_some() {
                    let token = job.cancelled.clone();
                    self.report["stage_at_cancel"] = job.stage.load(Ordering::Relaxed).into();
                    self.report["job_id_at_cancel"] = job.id.into();
                    self.report["revision_before_cancel"] = app.job_revision.into();
                    self.report["cancel_token_before"] = token.load(Ordering::Relaxed).into();
                    self.receiver = detach_receiver(app);
                    app.cancel_digitize(); // The unchanged handler used by the Cancel button.
                    self.report["cancel_handler_called"] = true.into();
                    self.report["cancel_token_after"] = token.load(Ordering::Relaxed).into();
                    self.report["revision_after_cancel"] = app.job_revision.into();
                    self.step = Step::Cancelled;
                }
            }
            Step::Cancelled => {
                match self
                    .receiver
                    .as_ref()
                    .expect("cancelled job receiver")
                    .try_recv()
                {
                    Err(mpsc::TryRecvError::Empty) => {}
                    result => {
                        self.report["worker_result_observed"] = true.into();
                        let cancelled = match result {
                            Ok(Err(error)) => {
                                let cancelled = error.to_ascii_lowercase().contains("cancel");
                                self.report["worker_error"] = error.into();
                                cancelled
                            }
                            Ok(Ok(_)) => {
                                self.report["worker_error"] = "Worker completed successfully after cancel; timing cannot prove interruption".into();
                                false
                            }
                            Err(_) => {
                                self.report["worker_error"] =
                                    "Worker disconnected without returning a result".into();
                                false
                            }
                        };
                        self.report["cancel_requested_before_completion"] = cancelled.into();
                        return self.finish(
                            app,
                            ctx,
                            if cancelled {
                                None
                            } else {
                                Some("Worker did not confirm cancellation")
                            },
                        );
                    }
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        false
    }
}

impl Editor {
    pub(super) fn capture_worker_probe(&mut self, ctx: &egui::Context) -> bool {
        let Some(mut probe) = self.capture_probe.take() else {
            return false;
        };
        if !probe.frame(self, ctx) {
            self.capture_probe = Some(probe);
        }
        true
    }
}
