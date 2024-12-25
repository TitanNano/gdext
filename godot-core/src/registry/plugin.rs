/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

#[cfg(all(since_api = "4.3", feature = "register-docs"))]
use crate::docs::*;
use crate::init::InitLevel;
use crate::meta::ClassName;
use crate::obj::Gd;
use crate::registry::class::GodotGetVirtual;
use crate::{classes, sys};
use std::any::Any;
use std::{any, fmt};
// TODO(bromeon): some information coming from the proc-macro API is deferred through PluginItem, while others is directly
// translated to code. Consider moving more code to the PluginItem, which allows for more dynamic registration and will
// be easier for a future builder API.

// ----------------------------------------------------------------------------------------------------------------------------------------------

/// Piece of information that is gathered by the self-registration ("plugin") system.
#[derive(Debug)]
pub struct ClassPlugin {
    pub class_name: ClassName,
    pub item: PluginItem,

    // Init-level is per ClassPlugin and not per PluginItem, because all components of all classes are mixed together in one
    // huge linker list. There is no per-class aggregation going on, so this allows to easily filter relevant classes.
    pub init_level: InitLevel,
}

/// Type-erased function object, holding a `register_class` function.
#[derive(Copy, Clone)]
pub struct ErasedRegisterFn {
    // Wrapper needed because Debug can't be derived on function pointers with reference parameters, so this won't work:
    // pub type ErasedRegisterFn = fn(&mut dyn std::any::Any);
    // (see https://stackoverflow.com/q/53380040)
    pub raw: fn(&mut dyn Any),
}

impl fmt::Debug for ErasedRegisterFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:0>16x}", self.raw as usize)
    }
}

#[derive(Copy, Clone)]
pub struct ErasedRegisterRpcsFn {
    pub raw: fn(&mut dyn Any),
}

impl fmt::Debug for ErasedRegisterRpcsFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:0>16x}", self.raw as usize)
    }
}

pub type ErasedDynifyFn = fn(Gd<classes::Object>) -> ErasedDynGd;

type GodotCreateFn = unsafe extern "C" fn(
    _class_userdata: *mut std::ffi::c_void,
    #[cfg(since_api = "4.4")] _notify_postinitialize: sys::GDExtensionBool,
) -> sys::GDExtensionObjectPtr;

#[derive(Clone, Debug)]
pub struct InherentImpl {
    /// Callback to library-generated function which registers functions and constants in the `impl` block.
    ///
    /// Always present since that's the entire point of this `impl` block.
    pub register_methods_constants_fn: ErasedRegisterFn,

    /// Callback to library-generated function which calls [`Node::rpc_config`](crate::classes::Node::rpc_config) for each function annotated with `#[rpc]` on the `impl` block.
    ///
    /// This function is called in [`UserClass::__before_ready()`](crate::obj::UserClass::__before_ready) definitions generated by the `#[derive(GodotClass)]` macro.
    pub register_rpcs_fn: Option<ErasedRegisterRpcsFn>,

    #[cfg(all(since_api = "4.3", feature = "register-docs"))]
    pub docs: InherentImplDocs,
}

/// Represents the data part of a [`ClassPlugin`] instance.
///
/// Each enumerator represents a different item in Rust code, which is processed by an independent proc macro (for example,
/// `#[derive(GodotClass)]` on structs, or `#[godot_api]` on impl blocks).
#[derive(Clone, Debug)]
pub enum PluginItem {
    /// Class definition itself, must always be available -- created by `#[derive(GodotClass)]`.
    Struct {
        base_class_name: ClassName,

        /// Godot low-level `create` function, wired up to library-generated `init`.
        generated_create_fn: Option<GodotCreateFn>,

        generated_recreate_fn: Option<
            unsafe extern "C" fn(
                p_class_userdata: *mut std::ffi::c_void,
                p_object: sys::GDExtensionObjectPtr,
            ) -> sys::GDExtensionClassInstancePtr,
        >,

        /// Callback to library-generated function which registers properties in the `struct` definition.
        register_properties_fn: ErasedRegisterFn,

        free_fn: unsafe extern "C" fn(
            _class_user_data: *mut std::ffi::c_void,
            instance: sys::GDExtensionClassInstancePtr,
        ),

        /// Calls `__before_ready()`, if there is at least one `OnReady` field. Used if there is no `#[godot_api] impl` block
        /// overriding ready.
        default_get_virtual_fn: Option<GodotGetVirtual>,

        /// Whether `#[class(tool)]` was used.
        is_tool: bool,

        /// Whether the base class is an `EditorPlugin`.
        is_editor_plugin: bool,

        /// Whether `#[class(internal)]` was used.
        is_internal: bool,

        /// Whether the class has a default constructor.
        is_instantiable: bool,

        #[cfg(all(since_api = "4.3", feature = "register-docs"))]
        docs: Option<StructDocs>,
    },

    /// Collected from `#[godot_api] impl MyClass`.
    InherentImpl(InherentImpl),

    /// Collected from `#[godot_api] impl I... for MyClass`.
    ITraitImpl {
        /// Virtual method documentation.
        #[cfg(all(since_api = "4.3", feature = "register-docs"))]
        virtual_method_docs: &'static str,

        /// Callback to user-defined `register_class` function.
        user_register_fn: Option<ErasedRegisterFn>,

        /// Godot low-level `create` function, wired up to the user's `init`.
        user_create_fn: Option<GodotCreateFn>,

        user_recreate_fn: Option<
            unsafe extern "C" fn(
                p_class_userdata: *mut ::std::os::raw::c_void,
                p_object: sys::GDExtensionObjectPtr,
            ) -> sys::GDExtensionClassInstancePtr,
        >,

        /// User-defined `to_string` function.
        user_to_string_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                r_is_valid: *mut sys::GDExtensionBool,
                r_out: sys::GDExtensionStringPtr,
            ),
        >,

        /// User-defined `on_notification` function.
        #[cfg(before_api = "4.2")]
        user_on_notification_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr, //
                p_what: i32,
            ),
        >,
        #[cfg(since_api = "4.2")]
        user_on_notification_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr, //
                p_what: i32,
                p_reversed: sys::GDExtensionBool,
            ),
        >,

        user_set_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                p_name: sys::GDExtensionConstStringNamePtr,
                p_value: sys::GDExtensionConstVariantPtr,
            ) -> sys::GDExtensionBool,
        >,

        user_get_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                p_name: sys::GDExtensionConstStringNamePtr,
                r_ret: sys::GDExtensionVariantPtr,
            ) -> sys::GDExtensionBool,
        >,

        /// Callback for other virtuals.
        get_virtual_fn: GodotGetVirtual,

        /// Callback for other virtuals.
        user_get_property_list_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                r_count: *mut u32,
            ) -> *const sys::GDExtensionPropertyInfo,
        >,

        // We do not support using this in Godot < 4.3, however it's easier to leave this in and fail elsewhere when attempting to use
        // this in Godot < 4.3.
        #[cfg(before_api = "4.3")]
        user_free_property_list_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                p_list: *const sys::GDExtensionPropertyInfo,
            ),
        >,
        #[cfg(since_api = "4.3")]
        user_free_property_list_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                p_list: *const sys::GDExtensionPropertyInfo,
                p_count: u32,
            ),
        >,

        user_property_can_revert_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                p_name: sys::GDExtensionConstStringNamePtr,
            ) -> sys::GDExtensionBool,
        >,

        user_property_get_revert_fn: Option<
            unsafe extern "C" fn(
                p_instance: sys::GDExtensionClassInstancePtr,
                p_name: sys::GDExtensionConstStringNamePtr,
                r_ret: sys::GDExtensionVariantPtr,
            ) -> sys::GDExtensionBool,
        >,
    },

    DynTraitImpl {
        /// TypeId of the `dyn Trait` object.
        dyn_trait_typeid: any::TypeId,

        /// Function that converts a `Gd<Object>` to a type-erased `DynGd<Object, dyn Trait>` (with the latter erased for common storage).
        erased_dynify_fn: fn(Gd<classes::Object>) -> ErasedDynGd,
    },
}

pub struct ErasedDynGd {
    pub boxed: Box<dyn Any>,
}
