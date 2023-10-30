use std::{cell::RefCell, rc::Rc};

use godot_ffi::{
    get_interface, GDExtensionCallErrorType, GDExtensionScriptInstanceDataPtr,
    GDExtensionScriptInstancePtr, VariantType,
};

use crate::{
    engine::{Object, Script, ScriptLanguage},
    godot_warn,
    obj::Gd,
};

use super::{
    meta::{MethodInfo, PropertyInfo},
    GodotString, StringName, Variant,
};

pub trait ScriptInstance {
    fn class_name(&self) -> GodotString;
    fn set(&mut self, name: StringName, value: &Variant) -> bool;
    fn get(&self, name: StringName) -> Option<Variant>;
    fn property_list(&self) -> Rc<Vec<PropertyInfo>>;
    fn method_list(&self) -> Rc<Vec<MethodInfo>>;
    fn call(
        &mut self,
        method: StringName,
        args: &[&Variant],
    ) -> Result<Variant, GDExtensionCallErrorType>;

    fn is_placeholder(&self) -> bool;
    fn has_method(&self, method: StringName) -> bool;
    fn get_script(&self) -> Gd<Script>;
    fn get_property_type(&self, name: StringName) -> VariantType;
    fn to_string(&self) -> GodotString;
    fn owner(&self) -> Gd<Object>;
    fn property_state(&self) -> Vec<(StringName, Variant)>;
    fn language(&self) -> Gd<ScriptLanguage>;
    fn refcount_decremented(&self) -> bool;
    fn refcount_incremented(&self);
}

impl<T: ScriptInstance + ?Sized> ScriptInstance for Box<T> {
    fn class_name(&self) -> GodotString {
        self.as_ref().class_name()
    }

    fn set(&mut self, name: StringName, value: &Variant) -> bool {
        self.as_mut().set(name, value)
    }

    fn get(&self, name: StringName) -> Option<Variant> {
        self.as_ref().get(name)
    }

    fn property_list(&self) -> Rc<Vec<PropertyInfo>> {
        self.as_ref().property_list()
    }

    fn method_list(&self) -> Rc<Vec<MethodInfo>> {
        self.as_ref().method_list()
    }

    fn call(
        &mut self,
        method: StringName,
        args: &[&Variant],
    ) -> Result<Variant, GDExtensionCallErrorType> {
        self.as_mut().call(method, args)
    }

    fn get_script(&self) -> Gd<Script> {
        self.as_ref().get_script()
    }

    fn is_placeholder(&self) -> bool {
        self.as_ref().is_placeholder()
    }

    fn has_method(&self, method: StringName) -> bool {
        self.as_ref().has_method(method)
    }

    fn get_property_type(&self, name: StringName) -> VariantType {
        self.as_ref().get_property_type(name)
    }

    fn to_string(&self) -> GodotString {
        self.as_ref().to_string()
    }

    fn owner(&self) -> Gd<Object> {
        self.as_ref().owner()
    }

    fn property_state(&self) -> Vec<(StringName, Variant)> {
        self.as_ref().property_state()
    }

    fn language(&self) -> Gd<ScriptLanguage> {
        self.as_ref().language()
    }

    fn refcount_incremented(&self) {
        self.as_ref().refcount_incremented()
    }

    fn refcount_decremented(&self) -> bool {
        self.as_ref().refcount_decremented()
    }
}

struct ScriptInstanceData<T: ScriptInstance> {
    inner: RefCell<T>,
}

pub fn create_script_instance<T: ScriptInstance>(rs_instance: T) -> GDExtensionScriptInstancePtr {
    godot_warn!("creating new script instance");

    let gd_instance = crate::sys::GDExtensionScriptInstanceInfo {
        set_func: Some(script_instance_info::set_func::<T>),
        get_func: Some(script_instance_info::get_func::<T>),
        get_property_list_func: Some(script_instance_info::get_property_list_func::<T>),
        free_property_list_func: Some(script_instance_info::free_property_list_func::<T>),
        // unimplemented until it's clear if it's needed
        property_get_revert_func: None,
        // unimplemented until it's clear if it's needed
        property_can_revert_func: None,
        get_owner_func: Some(script_instance_info::get_owner_func::<T>),
        get_property_state_func: Some(script_instance_info::get_property_state_func::<T>),
        get_script_func: Some(script_instance_info::get_script_func::<T>),
        get_language_func: Some(script_instance_info::get_language_func::<T>),
        get_fallback_func: None,
        get_method_list_func: Some(script_instance_info::get_method_list_func::<T>),
        get_property_type_func: Some(script_instance_info::get_property_type_func::<T>),
        free_func: Some(script_instance_info::free_func::<T>),
        free_method_list_func: Some(script_instance_info::free_method_list_func::<T>),
        has_method_func: Some(script_instance_info::has_method_func::<T>),
        call_func: Some(script_instance_info::call_func::<T>),
        // depreacted by Godot
        notification_func: None,
        to_string_func: Some(script_instance_info::to_string_func::<T>),
        refcount_decremented_func: Some(script_instance_info::refcount_decremented_func::<T>),
        refcount_incremented_func: Some(script_instance_info::refcount_incremented_func::<T>),
        is_placeholder_func: Some(script_instance_info::is_placeholder_func::<T>),
        set_fallback_func: None,
    };

    let data = ScriptInstanceData {
        inner: RefCell::new(rs_instance),
    };

    let inst = unsafe {
        let data_ptr = Box::into_raw(Box::new(data));
        let instance_ptr = Box::into_raw(Box::new(gd_instance));

        get_interface().script_instance_create.unwrap()(
            instance_ptr,
            data_ptr as GDExtensionScriptInstanceDataPtr,
        )
    };

    inst
}

mod script_instance_info {
    use std::{
        ffi::c_void,
        mem,
        ptr::drop_in_place,
        slice::{self},
    };

    use godot_ffi::{
        force_mut_ptr, GDExtensionBool, GDExtensionCallError, GDExtensionConstStringNamePtr,
        GDExtensionConstVariantPtr, GDExtensionInt, GDExtensionMethodInfo, GDExtensionObjectPtr,
        GDExtensionPropertyInfo, GDExtensionScriptInstanceDataPtr, GDExtensionScriptLanguagePtr,
        GDExtensionStringPtr, GDExtensionTypePtr, GDExtensionVariantPtr, GDExtensionVariantType,
        GodotFfi, PtrcallType, GDEXTENSION_CALL_OK,
    };

    use crate::builtin::{meta::GodotType, StringName, Variant};

    use super::{ScriptInstance, ScriptInstanceData};

    pub(super) unsafe extern "C" fn set_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        p_name: GDExtensionConstStringNamePtr,
        p_value: GDExtensionConstVariantPtr,
    ) -> GDExtensionBool {
        let instance = p_instance as *mut ScriptInstanceData<T>;
        let name = StringName::from_arg_ptr(p_name as GDExtensionTypePtr, PtrcallType::Standard);
        let value = &*Variant::ptr_from_sys(force_mut_ptr(p_value));

        let result = (*instance).inner.borrow_mut().set(name.clone(), value) as u8;

        result
    }

    pub(super) unsafe extern "C" fn get_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        p_name: GDExtensionConstStringNamePtr,
        r_ret: GDExtensionVariantPtr,
    ) -> GDExtensionBool {
        let instance = p_instance as *const ScriptInstanceData<T>;
        let name = StringName::from_arg_ptr(p_name as GDExtensionTypePtr, PtrcallType::Standard);

        let result = (*instance)
            .inner
            .borrow()
            .get(name.clone())
            .map(|variant| {
                variant.move_return_ptr(
                    r_ret as GDExtensionTypePtr,
                    godot_ffi::PtrcallType::Standard,
                )
            })
            .is_some();

        result as u8
    }

    pub(super) unsafe extern "C" fn get_property_list_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        r_count: *mut u32,
    ) -> *const GDExtensionPropertyInfo {
        let instance = p_instance as *const ScriptInstanceData<T>;

        let property_list: Vec<_> = (*instance)
            .inner
            .borrow()
            .property_list()
            .iter()
            .map(|prop| prop.property_sys())
            .collect();

        *r_count = property_list.len() as u32;

        let ptr = Box::into_raw(property_list.into_boxed_slice());

        (*ptr).as_ptr()
    }

    pub(super) unsafe extern "C" fn get_method_list_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        r_count: *mut u32,
    ) -> *const GDExtensionMethodInfo {
        let instance = p_instance as *const ScriptInstanceData<T>;

        let method_list: Vec<_> = (*instance)
            .inner
            .borrow()
            .method_list()
            .iter()
            .map(|method| method.method_sys())
            .collect();

        *r_count = method_list.len() as u32;

        let ptr = Box::into_raw(method_list.into_boxed_slice());

        (*ptr).as_ptr()
    }

    pub(super) unsafe extern "C" fn free_property_list_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        p_prop_info: *const GDExtensionPropertyInfo,
    ) {
        let instance = p_instance as *const ScriptInstanceData<T>;
        let length = (*instance).inner.borrow().property_list().len();

        let vec = Vec::from_raw_parts(p_prop_info as *mut GDExtensionPropertyInfo, length, length);

        drop(vec);
    }

    pub(super) unsafe extern "C" fn call_func<T: ScriptInstance>(
        p_self: GDExtensionScriptInstanceDataPtr,
        p_method: GDExtensionConstStringNamePtr,
        p_args: *const GDExtensionConstVariantPtr,
        p_argument_count: GDExtensionInt,
        r_return: GDExtensionVariantPtr,
        r_error: *mut GDExtensionCallError,
    ) {
        let instance = p_self as *mut ScriptInstanceData<T>;
        let method =
            StringName::from_arg_ptr(p_method as GDExtensionTypePtr, PtrcallType::Standard);

        let args: Vec<_> = slice::from_raw_parts(p_args, p_argument_count as usize)
            .iter()
            .map(|ptr| &*Variant::ptr_from_sys(force_mut_ptr(*ptr)))
            .collect();

        match (*instance)
            .inner
            .borrow_mut()
            .call(method.clone(), args.as_slice())
        {
            Ok(ret) => {
                ret.move_return_ptr(r_return as GDExtensionTypePtr, PtrcallType::Virtual);
                (*r_error).error = GDEXTENSION_CALL_OK;
            }

            Err(err) => (*r_error).error = err,
        };
    }

    pub(super) unsafe extern "C" fn get_script_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
    ) -> GDExtensionObjectPtr {
        let instance = p_instance as *const ScriptInstanceData<T>;

        let script = (*instance).inner.borrow().get_script();
        let ptr = script.obj_sys();

        mem::forget(script.into_ffi());

        ptr
    }

    pub(super) unsafe extern "C" fn is_placeholder_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
    ) -> u8 {
        let instance = p_instance as *const ScriptInstanceData<T>;

        (*instance).inner.borrow().is_placeholder() as u8
    }

    pub(super) unsafe extern "C" fn has_method_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        p_method: GDExtensionConstStringNamePtr,
    ) -> u8 {
        let instance = p_instance as *const ScriptInstanceData<T>;
        let method =
            StringName::from_arg_ptr(p_method as GDExtensionTypePtr, PtrcallType::Standard);

        let result = (*instance).inner.borrow().has_method(method.clone()) as u8;

        result
    }

    pub(super) unsafe extern "C" fn free_method_list_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        p_method_info: *const GDExtensionMethodInfo,
    ) {
        let instance = p_instance as *const ScriptInstanceData<T>;
        let length = (*instance).inner.borrow().method_list().len();

        let vec = Vec::from_raw_parts(force_mut_ptr(p_method_info), length, length);

        vec.into_iter().for_each(|method_info| {
            Vec::from_raw_parts(
                method_info.arguments,
                method_info.argument_count as usize,
                method_info.argument_count as usize,
            );

            Vec::from_raw_parts(
                method_info.default_arguments,
                method_info.default_argument_count as usize,
                method_info.default_argument_count as usize,
            );
        })
    }

    pub(super) unsafe extern "C" fn get_property_type_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        p_name: GDExtensionConstStringNamePtr,
        r_is_valid: *mut GDExtensionBool,
    ) -> GDExtensionVariantType {
        let instance = p_instance as *const ScriptInstanceData<T>;
        let name = StringName::from_arg_ptr(p_name as GDExtensionTypePtr, PtrcallType::Standard);

        let result = (*instance).inner.borrow().get_property_type(name.clone());

        *r_is_valid = true as u8;
        result.sys()
    }

    pub(super) unsafe extern "C" fn to_string_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        r_is_valid: *mut GDExtensionBool,
        r_str: GDExtensionStringPtr,
    ) {
        let instance = p_instance as *const ScriptInstanceData<T>;

        let Ok(inner) = (*instance).inner.try_borrow() else {
            // to_string of a script instance can be called when calling to_string
            // on the owning base object.

            return;
        };

        *r_is_valid = true as u8;

        inner
            .to_string()
            .move_return_ptr(r_str as GDExtensionTypePtr, PtrcallType::Standard);
    }

    pub(super) unsafe extern "C" fn get_owner_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
    ) -> GDExtensionObjectPtr {
        let instance = p_instance as *const ScriptInstanceData<T>;

        let owner = (*instance).inner.borrow().owner();

        let ptr = owner.obj_sys();

        mem::forget(owner);

        ptr
    }

    pub(super) unsafe extern "C" fn get_property_state_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
        add_prop: Option<
            unsafe extern "C" fn(
                name: GDExtensionConstStringNamePtr,
                value: GDExtensionConstVariantPtr,
                data: *mut c_void,
            ),
        >,
        data: *mut c_void,
    ) {
        let Some(add_prop) = add_prop else {
            return;
        };

        let instance = p_instance as *mut ScriptInstanceData<T>;

        let props = (*instance).inner.borrow().property_state();

        props.into_iter().for_each(|(name, value)| {
            add_prop(name.string_sys(), value.var_sys(), data);

            mem::forget(name);
            mem::forget(value);
        });
    }

    pub(super) unsafe extern "C" fn get_language_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
    ) -> GDExtensionScriptLanguagePtr {
        let instance = p_instance as *mut ScriptInstanceData<T>;

        let language = (*instance).inner.borrow().language().into_ffi();
        let ptr = language.sys() as GDExtensionScriptLanguagePtr;

        mem::forget(language);
        ptr
    }

    pub(super) unsafe extern "C" fn free_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
    ) {
        let instance_ptr = p_instance as *mut ScriptInstanceData<T>;

        drop_in_place(instance_ptr);
    }

    pub(super) unsafe extern "C" fn refcount_decremented_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
    ) -> GDExtensionBool {
        let instance = p_instance as *mut ScriptInstanceData<T>;

        (*instance).inner.borrow().refcount_decremented() as u8
    }

    pub(super) unsafe extern "C" fn refcount_incremented_func<T: ScriptInstance>(
        p_instance: GDExtensionScriptInstanceDataPtr,
    ) {
        let instance = p_instance as *mut ScriptInstanceData<T>;

        (*instance).inner.borrow().refcount_incremented()
    }
}
