// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The spike build swaps the app commands for its own.
#![cfg_attr(feature = "spike", allow(dead_code))]

mod analysis_cmds;
mod bloodstain_cmds;
mod camera_cmds;
mod commands;
mod crash_cmds;
mod crash_report;
mod diagram_cmds;
mod export_cmds;
mod guide_cmds;
mod license_cmds;
mod package_cmds;
mod photo_cmds;
mod register_cmds;
mod render_cmds;
mod scene3d_cmds;
mod scene_cmds;
#[cfg(feature = "spike")]
mod spike;

/// Ask hybrid-graphics laptops to run Locus on the dedicated GPU (NVIDIA Optimus and AMD
/// PowerXpress read these exported symbols; build.rs exports them). WebGL draws in
/// WebView2's own GPU process, which these don't reach, so tauri.conf.json also passes
/// `--force_high_performance_gpu` to WebView2. Help > About shows the GPU actually used.
#[cfg(windows)]
#[no_mangle]
#[used]
pub static NvOptimusEnablement: u32 = 1;
#[cfg(windows)]
#[no_mangle]
#[used]
pub static AmdPowerXpressRequestHighPerformance: u32 = 1;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::Emitter;

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default());

    #[cfg(not(feature = "spike"))]
    let builder = builder
        .invoke_handler(tauri::generate_handler![
            commands::project_create,
            commands::project_open,
            commands::import_preview,
            commands::import_commit,
            commands::evidence_verify,
            scene_cmds::scene_view,
            scene_cmds::analysis_state,
            scene_cmds::pick_resolve,
            scene_cmds::measure,
            scene_cmds::measurement_delete,
            scene_cmds::set_point_sigma,
            scene_cmds::cleanup_apply,
            scene_cmds::cleanup_preview,
            scene_cmds::cleanup_set_active,
            scene_cmds::app_info,
            scene_cmds::third_party_notices,
            scene_cmds::startup,
            register_cmds::registration_run,
            register_cmds::registration_edit,
            register_cmds::registrations,
            register_cmds::registration_apply,
            register_cmds::registration_report,
            diagram_cmds::diagrams,
            diagram_cmds::diagram_create,
            diagram_cmds::diagram_save,
            diagram_cmds::diagram_history,
            diagram_cmds::hand_solve,
            diagram_cmds::diagram_pdf,
            diagram_cmds::diagram_image,
            diagram_cmds::diagram_dxf,
            scene3d_cmds::scenes,
            scene3d_cmds::scene_create,
            scene3d_cmds::scene_save,
            scene3d_cmds::scene_history,
            scene3d_cmds::diagram_revision,
            scene3d_cmds::surface_at,
            scene3d_cmds::sun_position,
            scene3d_cmds::animation_evaluate,
            scene3d_cmds::animation_save,
            export_cmds::case_report,
            license_cmds::license_info,
            guide_cmds::guide_state,
            guide_cmds::sample_create,
            crash_report::crash_reports,
            crash_report::crash_reports_clear,
            crash_report::crash_report_view,
            license_cmds::license_install,
            package_cmds::package_export,
            package_cmds::package_open,
            package_cmds::package_file_open,
            export_cmds::measurements_csv,
            export_cmds::pointcloud_export,
            export_cmds::export_begin,
            export_cmds::export_bytes,
            render_cmds::render_start,
            render_cmds::render_frame,
            render_cmds::render_finish,
            render_cmds::render_cancel,
            analysis_cmds::trajectory_preview,
            analysis_cmds::trajectory_save,
            bloodstain_cmds::bloodstain_align,
            bloodstain_cmds::bloodstain_edges,
            bloodstain_cmds::bloodstain_stain,
            bloodstain_cmds::bloodstain_preview,
            bloodstain_cmds::bloodstain_save,
            camera_cmds::camera_preview,
            camera_cmds::camera_save,
            camera_cmds::witness_preview,
            camera_cmds::witness_save,
            crash_cmds::crash_mark_length,
            crash_cmds::crash_preview,
            crash_cmds::crash_save,
            crash_cmds::crash_crush_profile,
            photo_cmds::photo_setup,
            photo_cmds::photo_setup_set,
            photo_cmds::photo_sources,
            photo_cmds::photo_run,
            photo_cmds::photo_cancel,
            photo_cmds::photo_job,
            photo_cmds::photo_image,
            photo_cmds::photo_triangulate,
            photo_cmds::photo_scale,
            photo_cmds::photo_import,
            crash_cmds::stiffness_lookup,
            crash_cmds::stiffness_makes,
            analysis_cmds::analyses,
            analysis_cmds::analysis_withdraw,
            analysis_cmds::analysis_report,
            analysis_cmds::case_number,
            analysis_cmds::case_number_set,
            diagram_cmds::underlay_images,
            diagram_cmds::underlay_bytes,
            diagram_cmds::underlay_slice,
            diagram_cmds::underlay_calibrated,
        ])
        .register_asynchronous_uri_scheme_protocol("locus", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(scene_cmds::protocol(&app, request))
            });
        })
        .setup(|app| {
            use tauri::Manager;
            app.manage(scene_cmds::Builder::start(app.handle().clone()));
            app.manage(photo_cmds::PhotoState::default());
            license_cmds::load(app.handle());
            crash_report::install(app.handle());
            Ok(())
        });

    // Rendering spike: serve synthetic chunks; tauri.spike.conf.json opens spike.html.
    #[cfg(feature = "spike")]
    let builder = builder
        .invoke_handler(tauri::generate_handler![
            spike::spike_chunk,
            spike::spike_params,
            spike::spike_report
        ])
        .register_asynchronous_uri_scheme_protocol("spike", |_ctx, request, responder| {
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(spike::protocol(request))
            });
        })
        .setup(|_| Ok(spike::ensure_fixture()?));

    builder
        .menu(|app| {
            let item = |id: &str, text: &str, accel: Option<&str>| {
                MenuItem::with_id(app, id, text, true, accel)
            };
            Menu::with_items(
                app,
                &[
                    &Submenu::with_items(
                        app,
                        "File",
                        true,
                        &[
                            &item("new_project", "New Project…", Some("CmdOrCtrl+N"))?,
                            &item("open_project", "Open Project…", Some("CmdOrCtrl+O"))?,
                            &PredefinedMenuItem::separator(app)?,
                            &item("import", "Import Evidence…", Some("CmdOrCtrl+I"))?,
                            &item("verify_evidence", "Verify Evidence", None)?,
                            &PredefinedMenuItem::separator(app)?,
                            &PredefinedMenuItem::quit(app, None)?,
                        ],
                    )?,
                    &Submenu::with_items(
                        app,
                        "Edit",
                        true,
                        &[
                            &PredefinedMenuItem::cut(app, None)?,
                            &PredefinedMenuItem::copy(app, None)?,
                            &PredefinedMenuItem::paste(app, None)?,
                            &PredefinedMenuItem::select_all(app, None)?,
                        ],
                    )?,
                    &Submenu::with_items(
                        app,
                        "Scans",
                        true,
                        &[&item("register", "Register Scans…", Some("CmdOrCtrl+R"))?],
                    )?,
                    &Submenu::with_items(
                        app,
                        "Help",
                        true,
                        &[&item("about", "About Locus", None)?],
                    )?,
                ],
            )
        })
        // Menu actions are handled in the UI, which owns the dialogs.
        .on_menu_event(|app, event| {
            let _ = app.emit("menu", event.id().0.as_str());
        })
        .run(tauri::generate_context!())
        .expect("failed to start Locus");
}
