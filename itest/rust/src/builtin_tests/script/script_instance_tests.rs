/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::ffi::c_void;
use std::pin::Pin;

use godot::builtin::meta::{ClassName, FromGodot, MethodInfo, PropertyInfo, ToGodot};
use godot::builtin::{GString, StringName, Variant, VariantType};
use godot::engine::global::{MethodFlags, PropertyHint, PropertyUsageFlags};
use godot::engine::utilities::weakref;
use godot::engine::{
    create_script_instance, IScriptExtension, Object, Script, ScriptExtension, ScriptInstance,
    ScriptLanguage, WeakRef,
};
use godot::obj::{Base, Gd, WithBaseField};
use godot::register::{godot_api, GodotClass};
use godot::sys;
use godot::GdCell;

#[derive(GodotClass)]
#[class(base = ScriptExtension, init)]
struct TestScript {
    base: Base<ScriptExtension>,
}

#[godot_api]
impl IScriptExtension for TestScript {
    unsafe fn instance_create(&self, for_object: Gd<Object>) -> *mut c_void {
        create_script_instance(TestScriptInstance::new(self.to_gd().upcast(), for_object))
    }

    fn can_instantiate(&self) -> bool {
        true
    }
}

struct TestScriptInstance {
    /// A field to store the value of the `script_property_b` during tests.
    script_property_b: bool,
    prop_list: Vec<PropertyInfo>,
    method_list: Vec<MethodInfo>,
    script: Gd<Script>,
    owner: Gd<WeakRef>,
}

impl TestScriptInstance {
    fn new(script: Gd<Script>, owner: Gd<Object>) -> Self {
        Self {
            script,
            owner: weakref(owner.to_variant()).to(),
            script_property_b: false,
            prop_list: vec![PropertyInfo {
                variant_type: VariantType::Int,
                property_name: StringName::from("script_property_a"),
                class_name: ClassName::from_ascii_cstr("\0".as_bytes()),
                hint: PropertyHint::NONE,
                hint_string: GString::new(),
                usage: PropertyUsageFlags::NONE,
            }],

            method_list: vec![MethodInfo {
                id: 1,
                method_name: StringName::from("script_method_a"),
                class_name: ClassName::from_ascii_cstr("TestScript\0".as_bytes()),
                return_type: PropertyInfo {
                    variant_type: VariantType::String,
                    class_name: ClassName::none(),
                    property_name: StringName::from(""),
                    hint: PropertyHint::NONE,
                    hint_string: GString::new(),
                    usage: PropertyUsageFlags::NONE,
                },
                arguments: vec![
                    PropertyInfo {
                        variant_type: VariantType::String,
                        class_name: ClassName::none(),
                        property_name: StringName::from(""),
                        hint: PropertyHint::NONE,
                        hint_string: GString::new(),
                        usage: PropertyUsageFlags::NONE,
                    },
                    PropertyInfo {
                        variant_type: VariantType::Int,
                        class_name: ClassName::none(),
                        property_name: StringName::from(""),
                        hint: PropertyHint::NONE,
                        hint_string: GString::new(),
                        usage: PropertyUsageFlags::NONE,
                    },
                ],
                default_arguments: vec![],
                flags: MethodFlags::NORMAL,
            }],
        }
    }

    /// Turns the internal owner weakref into a strong ref.
    fn owner(&self) -> Gd<Object> {
        self.owner.get_ref().to()
    }

    /// Method of the test script and will be called during test runs.
    fn script_method_a(&self, arg_a: GString, arg_b: i32) -> String {
        format!("{arg_a}{arg_b}")
    }

    fn script_method_toggle_property_b(&mut self) -> bool {
        self.script_property_b = !self.script_property_b;
        true
    }
}

impl ScriptInstance for TestScriptInstance {
    fn class_name(_this: Pin<&GdCell<Self>>) -> GString {
        GString::from("TestScript")
    }

    fn set_property(this: Pin<&GdCell<Self>>, name: StringName, value: &Variant) -> bool {
        if name.to_string() == "script_property_b" {
            this.borrow_mut().unwrap().script_property_b = FromGodot::from_variant(value);
            true
        } else {
            false
        }
    }

    fn get_property(this: Pin<&GdCell<Self>>, name: StringName) -> Option<Variant> {
        match name.to_string().as_str() {
            "script_property_a" => Some(Variant::from(10)),
            "script_property_b" => Some(Variant::from(this.borrow().unwrap().script_property_b)),
            _ => None,
        }
    }

    fn get_property_list(this: Pin<&GdCell<Self>>) -> Box<[PropertyInfo]> {
        let guard = this.borrow().unwrap();

        guard.prop_list.as_slice().into()
    }

    fn get_method_list(this: Pin<&GdCell<Self>>) -> Box<[MethodInfo]> {
        let guard = this.borrow().unwrap();

        guard.method_list.as_slice().into()
    }

    fn call(
        this: Pin<&GdCell<Self>>,
        method: StringName,
        args: &[&Variant],
    ) -> Result<Variant, sys::GDExtensionCallErrorType> {
        match method.to_string().as_str() {
            "script_method_a" => {
                let arg_a = args[0].to::<GString>();
                let arg_b = args[1].to::<i32>();

                Ok(this
                    .borrow()
                    .unwrap()
                    .script_method_a(arg_a, arg_b)
                    .to_variant())
            }

            "script_method_toggle_property_b" => {
                let result = this.borrow_mut().unwrap().script_method_toggle_property_b();

                Ok(result.to_variant())
            }

            "script_method_re_entering" => {
                let mut guard = this.borrow_mut().unwrap();
                let mut owner = guard.owner();

                let inaccessible = this.make_inaccessible(&mut guard).unwrap();

                let result = owner.call("script_method_toggle_property_b".into(), &[]);

                drop(inaccessible);
                Ok(result)
            }

            _ => Err(sys::GDEXTENSION_CALL_ERROR_INVALID_METHOD),
        }
    }

    fn is_placeholder(_this: Pin<&GdCell<Self>>) -> bool {
        panic!("is_placeholder is not implemented")
    }

    fn has_method(_this: Pin<&GdCell<Self>>, method: StringName) -> bool {
        matches!(method.to_string().as_str(), "script_method_a")
    }

    fn get_script(this: Pin<&GdCell<Self>>) -> Gd<Script> {
        let guard = this.borrow().unwrap();

        guard.script.clone()
    }

    fn get_property_type(_this: Pin<&GdCell<Self>>, name: StringName) -> VariantType {
        match name.to_string().as_str() {
            "script_property_a" => VariantType::Int,
            _ => VariantType::Nil,
        }
    }

    fn to_string(_this: Pin<&GdCell<Self>>) -> GString {
        GString::from("script instance to string")
    }

    fn get_property_state(_this: Pin<&GdCell<Self>>) -> Vec<(StringName, Variant)> {
        panic!("property_state is not implemented")
    }

    fn get_language(_this: Pin<&GdCell<Self>>) -> Gd<ScriptLanguage> {
        panic!("language is not implemented")
    }

    fn on_refcount_decremented(_this: Pin<&GdCell<Self>>) -> bool {
        true
    }

    fn on_refcount_incremented(_this: Pin<&GdCell<Self>>) {}

    fn property_get_fallback(_this: Pin<&GdCell<Self>>, _name: StringName) -> Option<Variant> {
        None
    }

    fn property_set_fallback(
        _this: Pin<&GdCell<Self>>,
        _name: StringName,
        _value: &Variant,
    ) -> bool {
        false
    }
}
