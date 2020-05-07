//! Instantiate a module, call functions, and read exports.

use crate::{
    error::{update_last_error, CApiError},
    export::{wasmer_exports_t, wasmer_import_export_kind, NamedExports},
    import::wasmer_import_t,
    memory::wasmer_memory_t,
    value::wasmer_value_t,
    //value::{wasmer_value, wasmer_value_t, wasmer_value_tag},
    wasmer_result_t,
};
use libc::{c_char, c_int, c_void};
use std::collections::HashMap;
use std::ptr;
use std::slice;
use wasmer::{
    ExportType, Exports, Extern, Function, Global, ImportObject, Instance, Memory, Module, Table,
};

/// Opaque pointer to a `wasmer_runtime::Instance` value in Rust.
///
/// A `wasmer_runtime::Instance` represents a WebAssembly instance. It
/// is generally generated by the `wasmer_instantiate()` function, or by
/// the `wasmer_module_instantiate()` function for the most common paths.
#[repr(C)]
pub struct wasmer_instance_t;

/// Opaque pointer to a `wasmer_runtime::Ctx` value in Rust.
///
/// An instance context is passed to any host function (aka imported
/// function) as the first argument. It is necessary to read the
/// instance data or the memory, respectively with the
/// `wasmer_instance_context_data_get()` function, and the
/// `wasmer_instance_context_memory()` function.
///
/// It is also possible to get the instance context outside a host
/// function by using the `wasmer_instance_context_get()`
/// function. See also `wasmer_instance_context_data_set()` to set the
/// instance context data.
///
/// Example:
///
/// ```c
/// // A host function that prints data from the WebAssembly memory to
/// // the standard output.
/// void print(wasmer_instance_context_t *context, int32_t pointer, int32_t length) {
///     // Use `wasmer_instance_context` to get back the first instance memory.
///     const wasmer_memory_t *memory = wasmer_instance_context_memory(context, 0);
///
///     // Continue…
/// }
/// ```
#[repr(C)]
pub struct wasmer_instance_context_t;

/// Creates a new WebAssembly instance from the given bytes and imports.
///
/// The result is stored in the first argument `instance` if
/// successful, i.e. when the function returns
/// `wasmer_result_t::WASMER_OK`. Otherwise
/// `wasmer_result_t::WASMER_ERROR` is returned, and
/// `wasmer_last_error_length()` with `wasmer_last_error_message()` must
/// be used to read the error message.
///
/// The caller is responsible to free the instance with
/// `wasmer_instance_destroy()`.
///
/// Example:
///
/// ```c
/// // 1. Read a WebAssembly module from a file.
/// FILE *file = fopen("sum.wasm", "r");
/// fseek(file, 0, SEEK_END);
/// long bytes_length = ftell(file);
/// uint8_t *bytes = malloc(bytes_length);
/// fseek(file, 0, SEEK_SET);
/// fread(bytes, 1, bytes_length, file);
/// fclose(file);
///
/// // 2. Declare the imports (here, none).
/// wasmer_import_t imports[] = {};
///
/// // 3. Instantiate the WebAssembly module.
/// wasmer_instance_t *instance = NULL;
/// wasmer_result_t result = wasmer_instantiate(&instance, bytes, bytes_length, imports, 0);
///
/// // 4. Check for errors.
/// if (result != WASMER_OK) {
///     int error_length = wasmer_last_error_length();
///     char *error = malloc(error_length);
///     wasmer_last_error_message(error, error_length);
///     // Do something with `error`…
/// }
///
/// // 5. Free the memory!
/// wasmer_instance_destroy(instance);
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub unsafe extern "C" fn wasmer_instantiate(
    instance: *mut *mut wasmer_instance_t,
    wasm_bytes: *mut u8,
    wasm_bytes_len: u32,
    imports: *mut wasmer_import_t,
    imports_len: c_int,
) -> wasmer_result_t {
    if wasm_bytes.is_null() {
        update_last_error(CApiError {
            msg: "wasm bytes ptr is null".to_string(),
        });
        return wasmer_result_t::WASMER_ERROR;
    }
    let imports: &[wasmer_import_t] = slice::from_raw_parts(imports, imports_len as usize);
    let mut import_object = ImportObject::new();
    let mut namespaces = HashMap::new();
    for import in imports {
        let module_name = slice::from_raw_parts(
            import.module_name.bytes,
            import.module_name.bytes_len as usize,
        );
        let module_name = if let Ok(s) = std::str::from_utf8(module_name) {
            s
        } else {
            update_last_error(CApiError {
                msg: "error converting module name to string".to_string(),
            });
            return wasmer_result_t::WASMER_ERROR;
        };
        let import_name = slice::from_raw_parts(
            import.import_name.bytes,
            import.import_name.bytes_len as usize,
        );
        let import_name = if let Ok(s) = std::str::from_utf8(import_name) {
            s
        } else {
            update_last_error(CApiError {
                msg: "error converting import_name to string".to_string(),
            });
            return wasmer_result_t::WASMER_ERROR;
        };

        let namespace = namespaces.entry(module_name).or_insert_with(Exports::new);

        // TODO check that tag is actually in bounds here
        let export = match import.tag {
            wasmer_import_export_kind::WASM_MEMORY => {
                let mem = import.value.memory as *mut Memory;
                Extern::Memory((&*mem).clone())
            }
            wasmer_import_export_kind::WASM_FUNCTION => {
                let func_export = import.value.func as *mut Function;
                Extern::Function((&*func_export).clone())
            }
            wasmer_import_export_kind::WASM_GLOBAL => {
                let global = import.value.global as *mut Global;
                Extern::Global((&*global).clone())
            }
            wasmer_import_export_kind::WASM_TABLE => {
                let table = import.value.table as *mut Table;
                Extern::Table((&*table).clone())
            }
        };
        namespace.insert(import_name, export);
    }
    for (module_name, namespace) in namespaces.into_iter() {
        import_object.register(module_name, namespace);
    }

    let bytes: &[u8] = slice::from_raw_parts_mut(wasm_bytes, wasm_bytes_len as usize);
    let store = crate::get_global_store();

    let module_result = Module::from_binary(store, bytes);
    let module = match module_result {
        Ok(module) => module,
        Err(error) => {
            update_last_error(error);
            return wasmer_result_t::WASMER_ERROR;
        }
    };
    // TODO(mark): module is being freed here?  This looks like a mistake
    let result = Instance::new(&module, &import_object);
    let new_instance = match result {
        Ok(instance) => instance,
        Err(error) => {
            update_last_error(error);
            return wasmer_result_t::WASMER_ERROR;
        }
    };
    *instance = Box::into_raw(Box::new(new_instance)) as *mut wasmer_instance_t;
    wasmer_result_t::WASMER_OK
}

/// Returns the instance context. Learn more by looking at the
/// `wasmer_instance_context_t` struct.
///
/// This function returns `null` if `instance` is a null pointer.
///
/// Example:
///
/// ```c
/// const wasmer_instance_context_get *context = wasmer_instance_context_get(instance);
/// my_data *data = (my_data *) wasmer_instance_context_data_get(context);
/// // Do something with `my_data`.
/// ```
///
/// It is often useful with `wasmer_instance_context_data_set()`.
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_instance_context_get(
    instance: *mut wasmer_instance_t,
) -> *const wasmer_instance_context_t {
    if instance.is_null() {
        return ptr::null() as _;
    }

    unimplemented!("wasmer_instance_context_get: API changed")
    /*
    let instance = unsafe { &*(instance as *const Instance) };
    let context: *const Ctx = instance.context() as *const _;

    context as *const wasmer_instance_context_t
    */
}

/// Calls an exported function of a WebAssembly instance by `name`
/// with the provided parameters. The exported function results are
/// stored on the provided `results` pointer.
///
/// This function returns `wasmer_result_t::WASMER_OK` upon success,
/// `wasmer_result_t::WASMER_ERROR` otherwise. You can use
/// `wasmer_last_error_message()` to get the generated error message.
///
/// Potential errors are the following:
///
///   * `instance` is a null pointer,
///   * `name` is a null pointer,
///   * `params` is a null pointer.
///
/// Example of calling an exported function that needs two parameters, and returns one value:
///
/// ```c
/// // First argument.
/// wasmer_value_t argument_one = {
///     .tag = WASM_I32,
///     .value.I32 = 3,
/// };
///
/// // Second argument.
/// wasmer_value_t argument_two = {
///     .tag = WASM_I32,
///     .value.I32 = 4,
/// };
///
/// // First result.
/// wasmer_value_t result_one;
///
/// // All arguments and results.
/// wasmer_value_t arguments[] = {argument_one, argument_two};
/// wasmer_value_t results[]   = {result_one};
///
/// wasmer_result_t call_result = wasmer_instance_call(
///     instance,  // instance pointer
///     "sum",     // the exported function name
///     arguments, // the arguments
///     2,         // the number of arguments
///     results,   // the results
///     1          // the number of results
/// );
///
/// if (call_result == WASMER_OK) {
///     printf("Result is: %d\n", results[0].value.I32);
/// }
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub unsafe extern "C" fn wasmer_instance_call(
    instance: *mut wasmer_instance_t,
    name: *const c_char,
    params: *const wasmer_value_t,
    params_len: u32,
    results: *mut wasmer_value_t,
    results_len: u32,
) -> wasmer_result_t {
    unimplemented!("wasmer_instance_call: DynFunc not yet implemented!")
    /*
    if instance.is_null() {
        update_last_error(CApiError {
            msg: "instance ptr is null".to_string(),
        });

        return wasmer_result_t::WASMER_ERROR;
    }

    if name.is_null() {
        update_last_error(CApiError {
            msg: "name ptr is null".to_string(),
        });

        return wasmer_result_t::WASMER_ERROR;
    }

    if params.is_null() {
        update_last_error(CApiError {
            msg: "params ptr is null".to_string(),
        });

        return wasmer_result_t::WASMER_ERROR;
    }

    let params: &[wasmer_value_t] = slice::from_raw_parts(params, params_len as usize);
    let params: Vec<Val> = params.iter().cloned().map(|x| x.into()).collect();

    let func_name_c = CStr::from_ptr(name);
    let func_name_r = func_name_c.to_str().unwrap();

    let results: &mut [wasmer_value_t] = slice::from_raw_parts_mut(results, results_len as usize);
    let result = (&*(instance as *mut Instance)).call(func_name_r, &params[..]);

    match result {
        Ok(results_vec) => {
            if !results_vec.is_empty() {
                let ret = match results_vec[0] {
                    Val::I32(x) => wasmer_value_t {
                        tag: wasmer_value_tag::WASM_I32,
                        value: wasmer_value { I32: x },
                    },
                    Val::I64(x) => wasmer_value_t {
                        tag: wasmer_value_tag::WASM_I64,
                        value: wasmer_value { I64: x },
                    },
                    Val::F32(x) => wasmer_value_t {
                        tag: wasmer_value_tag::WASM_F32,
                        value: wasmer_value { F32: x },
                    },
                    Val::F64(x) => wasmer_value_t {
                        tag: wasmer_value_tag::WASM_F64,
                        value: wasmer_value { F64: x },
                    },
                    Val::V128(_) => unimplemented!("calling function with V128 parameter"),
                };
                results[0] = ret;
            }
            wasmer_result_t::WASMER_OK
        }
        Err(err) => {
            update_last_error(err);
            wasmer_result_t::WASMER_ERROR
        }
    }
    */
}

/// Gets all the exports of the given WebAssembly instance.
///

/// This function stores a Rust vector of exports into `exports` as an
/// opaque pointer of kind `wasmer_exports_t`.
///
/// As is, you can do anything with `exports` except using the
/// companion functions, like `wasmer_exports_len()`,
/// `wasmer_exports_get()` or `wasmer_export_kind()`. See the example below.
///
/// **Warning**: The caller owns the object and should call
/// `wasmer_exports_destroy()` to free it.
///
/// Example:
///
/// ```c
/// // Get the exports.
/// wasmer_exports_t *exports = NULL;
/// wasmer_instance_exports(instance, &exports);
///
/// // Get the number of exports.
/// int exports_length = wasmer_exports_len(exports);
/// printf("Number of exports: %d\n", exports_length);
///
/// // Read the first export.
/// wasmer_export_t *export = wasmer_exports_get(exports, 0);
///
/// // Get the kind of the export.
/// wasmer_import_export_kind export_kind = wasmer_export_kind(export);
///
/// // Assert it is a function (why not).
/// assert(export_kind == WASM_FUNCTION);
///
/// // Read the export name.
/// wasmer_byte_array name_bytes = wasmer_export_name(export);
///
/// assert(name_bytes.bytes_len == sizeof("sum") - 1);
/// assert(memcmp(name_bytes.bytes, "sum", sizeof("sum") - 1) == 0);
///
/// // Destroy the exports.
/// wasmer_exports_destroy(exports);
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub unsafe extern "C" fn wasmer_instance_exports(
    instance: *mut wasmer_instance_t,
    exports: *mut *mut wasmer_exports_t,
) {
    if instance.is_null() {
        return;
    }

    let instance_ref = &mut *(instance as *mut Instance);
    let mut exports_vec: Vec<ExportType> = instance_ref.module().exports().collect();

    let named_exports: Box<NamedExports> = Box::new(NamedExports(exports_vec));

    *exports = Box::into_raw(named_exports) as *mut wasmer_exports_t;
}

/// Sets the data that can be hold by an instance context.
///
/// An instance context (represented by the opaque
/// `wasmer_instance_context_t` structure) can hold user-defined
/// data. This function sets the data. This function is complementary
/// of `wasmer_instance_context_data_get()`.
///
/// This function does nothing if `instance` is a null pointer.
///
/// Example:
///
/// ```c
/// // Define your own data.
/// typedef struct {
///     // …
/// } my_data;
///
/// // Allocate them and set them on the given instance.
/// my_data *data = malloc(sizeof(my_data));
/// data->… = …;
/// wasmer_instance_context_data_set(instance, (void*) data);
///
/// // You can read your data.
/// {
///     my_data *data = (my_data*) wasmer_instance_context_data_get(wasmer_instance_context_get(instance));
///     // …
/// }
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_instance_context_data_set(
    instance: *mut wasmer_instance_t,
    data_ptr: *mut c_void,
) {
    unimplemented!(
        "wasmer_instance_context_data_set: API changed in a way that this is non-obvious"
    )
    /*
    if instance.is_null() {
        return;
    }

    let instance = unsafe { &mut *(instance as *mut Instance) };

    instance.context_mut().data = data_ptr;
    */
}

/// Gets the `memory_idx`th memory of the instance.
///
/// Note that the index is always `0` until multiple memories are supported.
///
/// This function is mostly used inside host functions (aka imported
/// functions) to read the instance memory.
///
/// Example of a _host function_ that reads and prints a string based on a pointer and a length:
///
/// ```c
/// void print_string(const wasmer_instance_context_t *context, int32_t pointer, int32_t length) {
///     // Get the 0th memory.
///     const wasmer_memory_t *memory = wasmer_instance_context_memory(context, 0);
///
///     // Get the memory data as a pointer.
///     uint8_t *memory_bytes = wasmer_memory_data(memory);
///
///     // Print what we assumed to be a string!
///     printf("%.*s", length, memory_bytes + pointer);
/// }
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_instance_context_memory(
    ctx: *const wasmer_instance_context_t,
    _memory_idx: u32,
) -> *const wasmer_memory_t {
    unimplemented!("wasmer_instance_context_memory: API changed")
    /*let ctx = unsafe { &*(ctx as *const Ctx) };
    let memory = ctx.memory(0);
    memory as *const Memory as *const wasmer_memory_t
        */
}

/// Gets the data that can be hold by an instance.
///
/// This function is complementary of
/// `wasmer_instance_context_data_set()`. Please read its
/// documentation. You can also read the documentation of
/// `wasmer_instance_context_t` to get other examples.
///
/// This function returns nothing if `ctx` is a null pointer.
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_instance_context_data_get(
    ctx: *const wasmer_instance_context_t,
) -> *mut c_void {
    unimplemented!("wasmer_instance_context_data_get: API changed")
    /*
    if ctx.is_null() {
        return ptr::null_mut() as _;
    }

    let ctx = unsafe { &*(ctx as *const Ctx) };

    ctx.data
    */
}

/// Frees memory for the given `wasmer_instance_t`.
///
/// Check the `wasmer_instantiate()` function to get a complete
/// example.
///
/// If `instance` is a null pointer, this function does nothing.
///
/// Example:
///
/// ```c
/// // Get an instance.
/// wasmer_instance_t *instance = NULL;
/// wasmer_instantiate(&instance, bytes, bytes_length, imports, 0);
///
/// // Destroy the instance.
/// wasmer_instance_destroy(instance);
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_instance_destroy(instance: *mut wasmer_instance_t) {
    if !instance.is_null() {
        unsafe { Box::from_raw(instance as *mut Instance) };
    }
}
