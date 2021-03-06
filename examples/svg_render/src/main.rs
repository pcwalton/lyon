extern crate cgmath;
extern crate gfx;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate lyon;
extern crate resvg;

extern crate svg_render;

use gfx::traits::{Device, FactoryExt};
use glutin::GlContext;
use lyon::tessellation::geometry_builder::{BuffersBuilder, VertexBuffers};
use lyon::tessellation::{FillOptions, FillTessellator, StrokeTessellator};
use resvg::tree::TreeExt;

use svg_render::FALLBACK_COLOR;
use svg_render::render::{self, fill_pipeline, ColorFormat, DepthFormat, Scene, VertexCtor};
use svg_render::{convert_path, convert_stroke};

const WINDOW_SIZE: f32 = 800.0;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        println!("Usage:\n\tsvg_render <file-name>");
        return;
    }

    let mut fill_tess = FillTessellator::new();
    let mut stroke_tess = StrokeTessellator::new();
    let mut mesh = VertexBuffers::new();

    let opt = resvg::Options::default();
    let rtree = resvg::parse_rtree_from_file(&args[1], &opt).unwrap();

    let view_box = rtree.svg_node().view_box;
    let mut transform = None;
    for node in rtree.root().descendants() {
        if let resvg::tree::NodeKind::Path(ref p) = *node.value() {
            // use the first transform component
            if transform == None {
                transform = Some(node.value().transform());
            }

            // get paint or create default one
            let (paint, opacity) = match p.fill {
                Some(f) => (f.paint, f.opacity),
                None => (resvg::tree::Paint::Color(FALLBACK_COLOR), 1.0),
            };

            // fall back to always use color fill
            // no gradients (yet?)
            let color = match paint {
                resvg::tree::Paint::Color(c) => c,
                _ => FALLBACK_COLOR,
            };

            let _ = fill_tess
                .tessellate_path(
                    convert_path(p).path_iter(),
                    &FillOptions::tolerance(0.01),
                    &mut BuffersBuilder::new(&mut mesh, VertexCtor::new(color, opacity)),
                )
                .expect("Error during tesselation!");

            if let Some(ref stroke) = p.stroke {
                let (stroke_color, stroke_opts) = convert_stroke(stroke);
                let opacity = stroke.opacity;
                let _ = stroke_tess.tessellate_path(
                    convert_path(p).path_iter(),
                    &stroke_opts.with_tolerance(0.01),
                    &mut BuffersBuilder::new(&mut mesh, VertexCtor::new(stroke_color, opacity)),
                );
            }
        }
    }

    println!(
        "Finished tesselation: {} vertices, {} indices",
        mesh.vertices.len(),
        mesh.indices.len()
    );
    println!("Use arrow keys to pan, quare brackes to zoom.");

    // get svg view box parameters
    let vb_width = view_box.size.width as f32;
    let vb_height = view_box.size.height as f32;
    let scale = vb_width / vb_height;

    // get x and y translation
    let (x_trans, y_trans) = if let Some(transform) = transform {
        (transform.e as f32, transform.f as f32)
    } else {
        (0.0, 0.0)
    };

    // set window scale
    let (width, height) = if scale < 1.0 {
        (WINDOW_SIZE, WINDOW_SIZE * scale)
    } else {
        (WINDOW_SIZE, WINDOW_SIZE / scale)
    };

    // init the scene object
    // use the viewBox, if available, to set the initial zoom and pan
    let pan = [vb_width / -2.0 + x_trans, vb_height / -2.0 + y_trans];
    let zoom = 2.0 / f32::max(vb_width, vb_height);
    let proj = cgmath::ortho(-scale, scale, -1.0, 1.0, -1.0, 1.0);
    let mut scene = Scene::new(zoom, pan, proj);

    println!("Original {:?}", scene);

    // set up event processing and rendering
    let mut event_loop = glutin::EventsLoop::new();
    let glutin_builder = glutin::WindowBuilder::new()
        .with_dimensions(width as u32, height as u32)
        .with_decorations(true)
        .with_title("SVG Renderer");

    let context = glutin::ContextBuilder::new().with_vsync(true);

    let (window, mut device, mut factory, mut main_fbo, mut main_depth) =
        gfx_window_glutin::init::<ColorFormat, DepthFormat>(glutin_builder, context, &event_loop);

    let shader = factory
        .link_program(
            render::VERTEX_SHADER.as_bytes(),
            render::FRAGMENT_SHADER.as_bytes(),
        )
        .unwrap();

    let pso = factory
        .create_pipeline_from_program(
            &shader,
            gfx::Primitive::TriangleList,
            gfx::state::Rasterizer::new_fill(),
            fill_pipeline::new(),
        )
        .unwrap();

    let (vbo, ibo) = factory.create_vertex_buffer_with_slice(&mesh.vertices[..], &mesh.indices[..]);

    let mut cmd_queue: gfx::Encoder<_, _> = factory.create_command_buffer().into();

    let constants = factory.create_constant_buffer(1);

    loop {
        if !update_inputs(&mut scene, &mut event_loop) {
            break;
        }

        gfx_window_glutin::update_views(&window, &mut main_fbo, &mut main_depth);

        cmd_queue.clear(&main_fbo.clone(), [1.0, 1.0, 1.0, 1.0]);

        cmd_queue.update_constant_buffer(&constants, &scene.into());
        cmd_queue.draw(
            &ibo,
            &pso,
            &fill_pipeline::Data {
                vbo: vbo.clone(),
                out_color: main_fbo.clone(),
                constants: constants.clone(),
            },
        );
        cmd_queue.flush(&mut device);

        window.swap_buffers().unwrap();

        device.cleanup();
    }
}

fn update_inputs(scene: &mut Scene, event_loop: &mut glutin::EventsLoop) -> bool {
    use glutin::Event;
    use glutin::VirtualKeyCode;
    use glutin::ElementState::Pressed;

    let mut status = true;

    event_loop.poll_events(|event| match event {
        Event::WindowEvent {
            event: glutin::WindowEvent::Closed,
            ..
        } => {
            println!("Window Closed!");
            status = false;
        }
        Event::WindowEvent {
            event: glutin::WindowEvent::Resized(w, h),
            ..
        } => {
            let scl = w as f32 / h as f32;
            scene.update_proj(cgmath::ortho(-scl, scl, -1.0, 1.0, -1.0, 1.0));
        }
        Event::WindowEvent {
            event:
                glutin::WindowEvent::KeyboardInput {
                    input:
                        glutin::KeyboardInput {
                            state: Pressed,
                            virtual_keycode: Some(key),
                            ..
                        },
                    ..
                },
            ..
        } => {
            println!("Preparing to update {:?}", scene);

            match key {
                VirtualKeyCode::Escape => {
                    println!("Closing");
                    status = false;
                }
                VirtualKeyCode::LBracket => {
                    scene.zoom *= 0.8;
                }
                VirtualKeyCode::RBracket => {
                    scene.zoom *= 1.2;
                }
                VirtualKeyCode::Left => {
                    scene.pan[0] -= 0.2 / scene.zoom;
                }
                VirtualKeyCode::Right => {
                    scene.pan[0] += 0.2 / scene.zoom;
                }
                VirtualKeyCode::Up => {
                    scene.pan[1] -= 0.2 / scene.zoom;
                }
                VirtualKeyCode::Down => {
                    scene.pan[1] += 0.2 / scene.zoom;
                }
                _key => {}
            };

            println!("Updated {:?}", scene);
        }
        _ => {}
    });

    status
}
