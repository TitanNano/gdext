use godot::builtin::Signal;
use godot::classes::{Engine, SceneTree};
use godot::global::godot_print;
use godot::tools::{godot_task, ToSignalFuture};

use crate::framework::itest;

async fn call_async_fn(signal: Signal) -> u8 {
    let value = 5;

    let _: () = signal.to_future().await;

    value + 5
}

#[itest]
fn start_async_task() {
    let tree = Engine::singleton()
        .get_main_loop()
        .unwrap()
        .cast::<SceneTree>();

    let signal = Signal::from_object_signal(&tree, "process_frame");

    godot_print!("starting godot_task...");
    godot_task(async move {
        godot_print!("running async task...");
        let result = call_async_fn(signal).await;
        godot_print!("got async result...");

        assert_eq!(result, 10);
        godot_print!("assertion done, async task complete!");
    });
    godot_print!("after godot_task...");
}
