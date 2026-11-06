//! Native workstation panels; document and stitch behavior remain shared.
use super::*;
use ui_theme::{Icon, icon_button};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayerEntry {
    Group {
        first: usize,
        count: usize,
        first_id: u64,
        color: [u8; 3],
        collapsed: bool,
    },
    Object {
        index: usize,
        grouped: bool,
    },
}

fn group_id(first_id: u64, color: [u8; 3]) -> egui::Id {
    egui::Id::new(("layer-colour-run", first_id, color))
}

/// Flatten only the visible presentation tree. Object indices always refer to
/// the original sewing order; equal colours in separate runs never coalesce.
fn layer_entries(
    objects: &[Object],
    mut collapsed: impl FnMut(u64, [u8; 3]) -> bool,
) -> Vec<LayerEntry> {
    let mut entries = Vec::with_capacity(objects.len());
    let mut first = 0;
    while first < objects.len() {
        let color = objects[first].color;
        let mut end = first + 1;
        while end < objects.len() && objects[end].color == color {
            end += 1;
        }
        let grouped = end - first >= 2;
        let closed = grouped && collapsed(objects[first].id, color);
        if grouped {
            entries.push(LayerEntry::Group {
                first,
                count: end - first,
                first_id: objects[first].id,
                color,
                collapsed: closed,
            });
        }
        if !closed {
            entries.extend((first..end).map(|index| LayerEntry::Object { index, grouped }));
        }
        first = end;
    }
    entries
}

fn colour_group_header(
    ui: &mut egui::Ui,
    color: [u8; 3],
    count: usize,
    collapsed: bool,
    selected: bool,
) -> egui::Response {
    let label = format!(
        "#{:02X}{:02X}{:02X} · {}",
        color[0], color[1], color[2], count
    );
    let response = ui.add_sized(
        [ui.available_width(), 28.],
        egui::Button::new("").selected(selected),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::CollapsingHeader,
            ui.is_enabled(),
            !collapsed,
            &label,
        )
    });
    if ui.is_rect_visible(response.rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        let foreground = visuals.fg_stroke.color;
        let center = response.rect.left_center() + Vec2::new(12., 0.);
        let points = if collapsed {
            [
                center + Vec2::new(-2., -4.),
                center + Vec2::new(2., 0.),
                center + Vec2::new(-2., 4.),
            ]
        } else {
            [
                center + Vec2::new(-4., -2.),
                center + Vec2::new(0., 2.),
                center + Vec2::new(4., -2.),
            ]
        };
        ui.painter().add(egui::Shape::line(
            points.to_vec(),
            Stroke::new(1.4_f32, foreground),
        ));
        let swatch = Rect::from_center_size(
            response.rect.left_center() + Vec2::new(30., 0.),
            Vec2::splat(10.),
        );
        ui.painter()
            .rect_filled(swatch, 1., Color32::from_rgb(color[0], color[1], color[2]));
        ui.painter().rect_stroke(
            swatch,
            1.,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            response.rect.left_center() + Vec2::new(42., 0.),
            egui::Align2::LEFT_CENTER,
            &label,
            egui::FontId::proportional(12.),
            foreground,
        );
    }
    response
}

impl Editor {
    pub(super) fn workspace_layers(&mut self, ctx: &egui::Context) {
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        let response=egui::SidePanel::left("objects").default_width(self.workspace.layers_width).width_range(180. ..=380.)
            .frame(egui::Frame::new().fill(colors.panel).inner_margin(10))
            .show(ctx,|ui| {
                if self.decision_pending() { ui.disable(); }
                ui.horizontal(|ui| {
                    ui.strong(language.text("Objects"));
                    ui.label(egui::RichText::new(self.project.objects.len().to_string()).color(colors.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        if icon_button(ui,Icon::Layers,true,true,language.text("Hide objects panel")).clicked(){self.workspace.layers_open=false;}
                    });
                });
                ui.small(language.text("Sewing order · top to bottom")).on_hover_text(language.text("Click: one layer · Shift-click: range · Ctrl-click: toggle · Ctrl+A: all layers"));
                ui.add_space(6.);
                let mut visibility=None;let mut lock=None;let mut target=None;let mut context_responses=vec![];
                let count=self.project.objects.len();
                // Measure localized empty guidance before allocating the list,
                // so wrapping cannot consume the footer's reserved space.
                if count==0 {ui.add_space(12.);ui.label(language.text(if self.stitch_only{"Machine preview has no vector objects."}else{"Your objects will appear here."}));ui.small(language.text("Import artwork or draw a contour to begin."));}
                let entries=ctx.data_mut(|data|layer_entries(&self.project.objects,|first_id,color|data.get_persisted::<bool>(group_id(first_id,color)).unwrap_or(false)));
                egui::ScrollArea::vertical().id_salt("objects-scroll").auto_shrink([false,false])
                    .max_height((ui.available_height()-92.).max(70.))
                    .show_rows(ui,28.,entries.len(),|ui,range| {
                        for entry in &entries[range] {
                            let (i,grouped)=match *entry {
                                LayerEntry::Group{first,count,first_id,color,collapsed}=>{
                                    let selected=(first..first+count).contains(&self.selected)||self.project.objects[first..first+count].iter().any(|o|self.merge_selection.contains(&o.id));
                                    let header=ui.push_id(group_id(first_id,color),|ui|colour_group_header(ui,color,count,collapsed,selected)).inner;
                                    if header.has_focus()||header.clicked(){self.layer_focus_id=Some(header.id);}
                                    if header.on_hover_text(language.text("Consecutive same-colour objects. View grouping preserves sewing order.")).clicked(){ctx.data_mut(|data|data.insert_persisted(group_id(first_id,color),!collapsed));ctx.request_repaint();}
                                    continue;
                                }
                                LayerEntry::Object{index,grouped}=>(index,grouped),
                            };
                            let object=&self.project.objects[i];
                            let selected=i==self.selected||self.merge_selection.contains(&object.id);
                            ui.push_id(("layer-object",object.id),|ui| {
                            egui::Frame::new().fill(if selected{colors.selection}else{Color32::TRANSPARENT}).corner_radius(3).show(ui,|ui| {
                                ui.spacing_mut().item_spacing.x=4.;
                                ui.horizontal(|ui| {
                                    if grouped{ui.add_space(10.);}
                                    if icon_button(ui,if object.visible{Icon::Eye}else{Icon::Hidden},false,!self.stitch_only,language.text(if object.visible {"Exclude object from generated stitches"} else {"Include object in generated stitches"})).clicked(){visibility=Some(i);}
                                    let (rect,_)=ui.allocate_exact_size(Vec2::new(10.,16.),Sense::hover());
                                    ui.painter().rect_filled(rect.shrink2(Vec2::new(0.,2.)),2.,Color32::from_rgb(object.color[0],object.color[1],object.color[2]));
                                    ui.painter().rect_stroke(rect.shrink2(Vec2::new(0.,2.)),2.,Stroke::new(0.7_f32,colors.border),egui::StrokeKind::Inside);
                                    let show_kind=ui.available_width()>140.;
                                    let width=(ui.available_width()-32.-if show_kind{45.}else{0.}).max(35.);
                                    let row=ui.add_sized([width,26.],egui::Label::new(&object.name).truncate().sense(Sense::click()));
                                    row.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel,ui.is_enabled(),selected,&object.name));
                                    let row=row.on_hover_text(format!("{} · {} · #{}",object.name,stitch_kind_label(language,object.settings.kind),object.id));
                                    if row.has_focus()||row.clicked(){self.layer_focus_id=Some(row.id);}
                                    if row.clicked(){target=Some((i,ui.input(|input|input.modifiers)));}
                                    context_responses.push((i,row));
                                    if show_kind{ui.small(stitch_kind_label(language,object.settings.kind));}
                                    let locked=self.locked_objects.contains(&object.id);
                                    if icon_button(ui,if locked{Icon::Lock}else{Icon::Unlock},locked,!self.stitch_only,language.text(if locked{"Unlock object editing"}else{"Lock editing in this workspace"})).clicked(){lock=Some(object.id);}
                                });
                            });
                            });
                        }
                    });
                if let Some(i)=visibility{self.snapshot();self.project.objects[i].visible= !self.project.objects[i].visible;self.changed();}
                if let Some(id)=lock{if !self.locked_objects.remove(&id){self.locked_objects.insert(id);}self.handle=None;}
                if let Some((i,modifiers))=target{self.select_layer(i,modifiers);}
                for (i,response) in context_responses {if response.secondary_clicked(){self.select_object(i);}self.object_context_menu(&response);}
                ui.separator();
                ui.horizontal(|ui| {
                    let available=!self.stitch_only&&!self.selected_locked();
                    let earlier=available&&self.selected<count&&self.selected.checked_sub(1).and_then(|i|self.project.objects.get(i)).is_some_and(|o|!self.locked_objects.contains(&o.id));
                    let later=available&&self.selected.checked_add(1).and_then(|i|self.project.objects.get(i)).is_some_and(|o|!self.locked_objects.contains(&o.id));
                    if icon_button(ui,Icon::Up,false,earlier,language.text("Earlier")).clicked(){self.move_selected(false);}
                    if icon_button(ui,Icon::Down,false,later,language.text("Later")).clicked(){self.move_selected(true);}
                    ui_theme::icon_menu(ui,Icon::Add,language.text("Object actions"),|ui| {
                        if ui.add_enabled(!self.stitch_only,egui::Button::new(language.text("Rectangle"))).clicked(){self.add_object(false);ui.close();}
                        if ui.add_enabled(!self.stitch_only,egui::Button::new(language.text("Bezier"))).clicked(){self.add_object(true);ui.close();}
                        if ui.add_enabled(count>1&&self.locked_objects.is_empty(),egui::Button::new(language.text("Optimize same-color order"))).clicked(){self.snapshot();engine::optimize_order(&mut self.project);self.selected=0;self.changed();ui.close();}
                    });
                    if icon_button(ui,Icon::Trash,false,available&&self.selected<count,language.text("Remove object")).clicked(){self.remove_selected();}
                });
                if !self.merge_selection.is_empty(){ui.small(language.message(&format!("Selected areas: {}",self.merge_selection.len())));}
            });
        self.workspace.layers_width = response.response.rect.width();
    }

    pub(super) fn workspace_inspector(&mut self, ctx: &egui::Context) {
        let language = self.language;
        let colors = self.workspace.theme.tokens();
        let response=egui::SidePanel::right("parameters").default_width(self.workspace.inspector_width).width_range(240. ..=420.)
            .frame(egui::Frame::new().fill(colors.panel).inner_margin(12))
            .show(ctx,|ui| {
                if self.decision_pending() { ui.disable(); }
                ui.horizontal(|ui| {
                    ui.strong(language.text("Inspector"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        if icon_button(ui,Icon::Inspector,true,true,language.text("Hide inspector")).clicked(){self.workspace.inspector_open=false;}
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt("inspector-scroll").auto_shrink([false,false]).show(ui,|ui| {
                    self.object_parameters(ui);
                    self.underlay_parameters(ui);
                    if !self.stitch_only && self.selected<self.project.objects.len() {
                        ui.add_space(10.);
                        ui.add_enabled_ui(!self.selected_locked(),|ui| {
                            ui.collapsing(language.text("Geometry tools"),|ui| {
                                ui.collapsing(language.text("Centerlines"),|ui| {
                                    ui.label(language.text("Pixel size (mm)"));
                                    ui.add(egui::DragValue::new(&mut self.centerline_pixel_mm).speed(0.01).range(0.05..=2.).fixed_decimals(2));
                                    ui.small(language.text("For narrow areas and lettering. Creates Running paths and keeps the original hidden."));
                                    let available=self.project.objects.get(self.selected).is_some_and(|o|o.running_paths.is_empty()&&(!o.contours.is_empty()||!o.bezier_segments.is_empty()||o.satin_column.is_some()));
                                    if ui.add_enabled(available&&self.centerline_job.is_none(),egui::Button::new(language.text("Create running copy"))).clicked(){self.create_centerlines();}
                                });
                                self.geometry_tools(ui);
                            });
                        });
                    }
                    ui.add_enabled_ui(!self.stitch_only,|ui| {
                        self.layering_profile_ui(ui);
                        self.machine_profile_ui(ui);
                    });
                    if self.stitch_only {
                        ui.small(language.text("Imported stitches cannot be regenerated. Open an editable project to change machine limits."));
                    }
                    ui.collapsing(language.text("Quality checks"),|ui|self.quality_controls(ui));
                    ui.add_space(10.);
                    ui.collapsing(language.text("Artwork"),|ui| {
                        ui.label(language.text("Next import width (mm)"));
                        ui.add(egui::DragValue::new(&mut self.import_width_mm).speed(1.).range(5. ..=400.));
                        if self.project.source_image.is_some()&&ui.button(language.text("Redigitize saved source…")).clicked(){self.redigitize_source(ctx);}
                        ui.small(language.text("DST preview colors are illustrative."));
                    });
                });
            });
        self.workspace.inspector_width = response.response.rect.width();
    }

    fn layering_profile_ui(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let before = self.project.layering.clone();
        ui.collapsing(language.text("Layer stitching"),|ui| {
            ui.checkbox(&mut self.project.layering.enabled,language.text("Remove hidden lower fills"));
            ui.add_enabled_ui(self.project.layering.enabled,|ui| {
                ui.label(language.text("Registration overlap (mm)"));
                ui.add(egui::DragValue::new(&mut self.project.layering.overlap_mm)
                    .range(0. ..=5.).speed(0.05).max_decimals(2).suffix(" mm"));
            });
            ui.small(language.text("Later filled objects cover earlier ones. Only generated stitches are cut back; editable contours and object order are preserved."));
        });
        if before != self.project.layering {
            if self.field_before.is_none() {
                let mut snapshot = self.project.clone();
                snapshot.layering = before;
                self.field_before = Some(snapshot);
            }
            self.changed();
        }
    }
}

#[cfg(test)]
mod hierarchy_tests {
    use super::*;

    fn objects(colors: &[[u8; 3]]) -> Vec<Object> {
        colors
            .iter()
            .enumerate()
            .map(|(i, &color)| Object {
                id: 10 + i as u64,
                color,
                name: format!("Object {i}"),
                ..Object::default()
            })
            .collect()
    }

    #[test]
    fn grouping_is_stable_under_names_visibility_and_geometry_edits() {
        let red = [180, 20, 30];
        let blue = [20, 40, 180];
        let mut source = objects(&[red, red, blue]);
        let before = serde_json::to_value(&source).unwrap();
        let entries = layer_entries(&source, |_, _| false);
        assert_eq!(
            entries,
            vec![
                LayerEntry::Group {
                    first: 0,
                    count: 2,
                    first_id: 10,
                    color: red,
                    collapsed: false
                },
                LayerEntry::Object {
                    index: 0,
                    grouped: true
                },
                LayerEntry::Object {
                    index: 1,
                    grouped: true
                },
                LayerEntry::Object {
                    index: 2,
                    grouped: false
                },
            ]
        );
        assert_eq!(serde_json::to_value(&source).unwrap(), before);
        source[0].name = "A much longer renamed object".into();
        source[1].visible = false;
        source[1].contours = vec![vec![
            Point::new(0., 0.),
            Point::new(3., 0.),
            Point::new(1., 2.),
        ]];
        assert_eq!(layer_entries(&source, |_, _| false), entries);
        let collapsed = layer_entries(&source, |id, color| id == 10 && color == red);
        assert_eq!(collapsed.len(), 2);
        assert_eq!(
            collapsed[1],
            LayerEntry::Object {
                index: 2,
                grouped: false
            }
        );
    }

    #[test]
    fn red_blue_red_runs_and_object_identity_keep_exact_sewing_order() {
        let red = [180, 20, 30];
        let blue = [20, 40, 180];
        let source = objects(&[red, red, blue, blue, red, red]);
        let entries = layer_entries(&source, |_, _| false);
        let header_ids: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                LayerEntry::Group { first_id, .. } => Some(*first_id),
                _ => None,
            })
            .collect();
        let object_ids: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                LayerEntry::Object { index, .. } => Some(source[*index].id),
                _ => None,
            })
            .collect();
        assert_eq!(header_ids, [10, 12, 14]);
        assert_eq!(object_ids, source.iter().map(|o| o.id).collect::<Vec<_>>());
        let collapsed = layer_entries(&source, |id, _| id == 10);
        assert!(collapsed.contains(&LayerEntry::Object {
            index: 4,
            grouped: true
        }));
        assert!(collapsed.contains(&LayerEntry::Object {
            index: 5,
            grouped: true
        }));
        let singletons = objects(&[red, blue, red]);
        assert_eq!(
            layer_entries(&singletons, |_, _| true),
            (0..3)
                .map(|index| LayerEntry::Object {
                    index,
                    grouped: false
                })
                .collect::<Vec<_>>()
        );
        assert_ne!(group_id(10, red), group_id(14, red));
    }
}
