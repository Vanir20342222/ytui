//! Android-only platform glue. All application screens and controls are Rust.
use super::*;
use android_activity::AndroidApp;
use glow::HasContext;
use jni::{
    JavaVM,
    objects::{GlobalRef, JClass, JObject, JValue},
};
static CONTEXT: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_ytui_android_NativeBridge_frameAvailable(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    if let Some(ctx) = CONTEXT.lock().expect("context").as_ref() {
        ctx.request_repaint();
    }
}
const TEXTURE_EXTERNAL_OES: u32 = 0x8D65;
static BRIDGE: OnceLock<(JavaVM, GlobalRef)> = OnceLock::new();

pub(crate) fn initialize_bridge(env: &mut jni::JNIEnv) -> Result<()> {
    if BRIDGE.get().is_none() {
        let vm = env.get_java_vm()?;
        let class = env.find_class("dev/ytui/android/MediaBridge")?;
        let global = env.new_global_ref(class)?;
        let _ = BRIDGE.set((vm, global));
    }
    yt_dlp_rs::set_embedded_js_evaluator(|script| {
        let result = (|| -> Result<String> {
            let (vm, class) = BRIDGE.get().context("Missing Android JS bridge")?;
            let mut env = vm.attach_current_thread()?;
            let class: &JClass = class.as_obj().into();
            let text = env.new_string(script)?;
            let value = match env.call_static_method(
                class,
                "evaluate",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&text)],
            ) {
                Ok(value) => value.l()?,
                Err(error) => {
                    let _ = env.exception_clear();
                    return Err(error.into());
                }
            };
            let result: String = env.get_string((&value).into())?.into();
            Ok(result)
        })();
        result.map_err(|_| "Android isolated JavaScript solver failed or timed out".into())
    });
    Ok(())
}

fn bridge_call(method: &str, text: Option<&str>) -> Result<String> {
    let (vm, class) = BRIDGE.get().context("Android bridge is not initialized")?;
    let mut env = vm.attach_current_thread()?;
    let class: &JClass = class.as_obj().into();
    if let Some(text) = text {
        let s = env.new_string(text)?;
        env.call_static_method(
            class,
            method,
            "(Ljava/lang/String;)V",
            &[JValue::Object(&s)],
        )?;
        Ok(String::new())
    } else {
        let s = env
            .call_static_method(class, method, "()Ljava/lang/String;", &[])?
            .l()?;
        Ok(env.get_string((&s).into())?.into())
    }
}
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let result = (|| -> Result<()> {
        // Android owns both pointers for the activity/process lifetime.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let loader = env
            .call_method(
                &activity,
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?;
        let name = env.new_string("dev.ytui.android.MediaBridge")?;
        let class = env
            .call_method(
                loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&name)],
            )?
            .l()?;
        let class = env.new_global_ref(class)?;
        drop(env);
        let _ = BRIDGE.set((vm, class));
        if ENGINE.get().is_none() {
            let root = app.internal_data_path().context("No app data directory")?;
            initialize(&root, &root.join("cache"))?;
        }
        let engine = ENGINE.get().context("No engine")?.clone();
        let options = eframe::NativeOptions {
            android_app: Some(app),
            renderer: eframe::Renderer::Glow,
            vsync: true,
            ..Default::default()
        };
        eframe::run_native(
            "ytui",
            options,
            Box::new(move |cc| {
                *CONTEXT.lock().expect("context") = Some(cc.egui_ctx.clone());
                Ok(Box::new(AndroidUi {
                    app: ui::MobileApp::new(engine.clone(), &cc.egui_ctx),
                    last: std::time::Instant::now(),
                }))
            }),
        )
        .map_err(|e| anyhow::anyhow!("Android UI: {e}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!(
            "ytui startup: {}",
            ytui_providers::diagnostic(&format!("{error:#}"))
        );
    }
}
struct AndroidUi {
    app: ui::MobileApp,
    last: std::time::Instant,
}
impl eframe::App for AndroidUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last.elapsed() > std::time::Duration::from_millis(150) {
            if let Ok(raw) = bridge_call("snapshot", None)
                && let Ok(value) = serde_json::from_str(&raw)
            {
                self.app.platform_status(ctx, value);
            }
            self.last = std::time::Instant::now();
        }
        self.app.frame(ctx);
        for effect in std::mem::take(&mut self.app.effects) {
            let request = match effect {
                ui::Effect::Play(value) => json!({"op":"play","request":value}),
                ui::Effect::Download(value) => json!({"op":"download","request":value}),
                ui::Effect::Command(value) => value,
                ui::Effect::Login => json!({"op":"login"}),
                ui::Effect::Pick(kind) => json!({"op":"pick","kind":kind}),
                ui::Effect::Open(url) => json!({"op":"open","url":url}),
                ui::Effect::Offline(id) => json!({"op":"offline","id":id}),
            };
            if let Err(error) = bridge_call("execute", Some(&request.to_string())) {
                self.app.status = error.to_string();
            }
        }
    }
    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        if let Some(gl) = gl {
            VIDEO.with(|slot| {
                if let Some(video) = slot.borrow_mut().take() {
                    unsafe {
                        gl.delete_program(video.program);
                        gl.delete_buffer(video.vertices);
                        gl.delete_vertex_array(video.vao);
                        gl.delete_texture(video.texture);
                    }
                }
            });
        }
        *CONTEXT.lock().expect("context") = None;
        let _ = bridge_call("releaseVideo", Some(""));
    }
}
struct Video {
    program: glow::Program,
    vertices: glow::Buffer,
    vao: glow::VertexArray,
    texture: glow::Texture,
}
thread_local! {static VIDEO:std::cell::RefCell<Option<Video>>=const{std::cell::RefCell::new(None)};}
impl Video {
    unsafe fn new(gl: &glow::Context) -> Result<Self> {
        // Every operation runs in egui's current GLES context on the render thread.
        unsafe {
            let program = gl.create_program().map_err(anyhow::Error::msg)?;
            for (kind, source) in [
                (
                    glow::VERTEX_SHADER,
                    "attribute vec2 a_pos; varying vec2 v_uv; uniform mat4 u_transform; void main(){gl_Position=vec4(a_pos,0.0,1.0);v_uv=(u_transform*vec4((a_pos+1.0)*0.5,0.0,1.0)).xy;}",
                ),
                (
                    glow::FRAGMENT_SHADER,
                    "#extension GL_OES_EGL_image_external : require\nprecision mediump float; varying vec2 v_uv; uniform samplerExternalOES u_video; void main(){gl_FragColor=texture2D(u_video,v_uv);}",
                ),
            ] {
                let shader = gl.create_shader(kind).map_err(anyhow::Error::msg)?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                ensure!(
                    gl.get_shader_compile_status(shader),
                    "Video shader: {}",
                    gl.get_shader_info_log(shader)
                );
                gl.attach_shader(program, shader);
                gl.delete_shader(shader);
            }
            gl.link_program(program);
            ensure!(
                gl.get_program_link_status(program),
                "Video shader link failed"
            );
            let vao = gl.create_vertex_array().map_err(anyhow::Error::msg)?;
            gl.bind_vertex_array(Some(vao));
            let vertices = gl.create_buffer().map_err(anyhow::Error::msg)?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertices));
            let data: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            let attribute = gl
                .get_attrib_location(program, "a_pos")
                .context("Video shader attribute missing")?;
            gl.enable_vertex_attrib_array(attribute);
            gl.vertex_attrib_pointer_f32(attribute, 2, glow::FLOAT, false, 8, 0);
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            gl.bind_texture(TEXTURE_EXTERNAL_OES, Some(texture));
            gl.tex_parameter_i32(
                TEXTURE_EXTERNAL_OES,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                TEXTURE_EXTERNAL_OES,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                TEXTURE_EXTERNAL_OES,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                TEXTURE_EXTERNAL_OES,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            Ok(Self {
                program,
                vertices,
                vao,
                texture,
            })
        }
    }
    unsafe fn paint(&self, gl: &glow::Context) -> Result<()> {
        let (vm, class) = BRIDGE.get().context("No media bridge")?;
        let mut env = vm.attach_current_thread()?;
        let class: &JClass = class.as_obj().into();
        let matrix = env
            .call_static_method(
                class,
                "videoFrame",
                "(I)[F",
                &[JValue::Int(self.texture.0.get() as i32)],
            )?
            .l()?;
        if matrix.is_null() {
            return Ok(());
        }
        let matrix: jni::objects::JFloatArray = matrix.into();
        let mut transform = [0.0_f32; 16];
        env.get_float_array_region(&matrix, 0, &mut transform)?;
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(TEXTURE_EXTERNAL_OES, Some(self.texture));
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "u_video").as_ref(), 0);
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(self.program, "u_transform")
                    .as_ref(),
                false,
                &transform,
            );
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
        Ok(())
    }
}
pub fn paint_video(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.painter().add(egui::PaintCallback {
        rect,
        callback: Arc::new(eframe::egui_glow::CallbackFn::new(|_info, painter| {
            VIDEO.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.is_none() {
                    *slot = unsafe { Video::new(painter.gl()).ok() };
                }
                if let Some(video) = slot.as_ref() {
                    let _ = unsafe { video.paint(painter.gl()) };
                }
            });
        })),
    });
}
