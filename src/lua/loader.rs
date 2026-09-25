//! Install the controlled `require` that reads only the captured source bundle.
use super::error;
use crate::{
    error::Result,
    runtime::ExecutionContext,
    source::{SourceBundle, names},
};
use mlua::{Lua, RegistryKey, Value};
use std::{cell::RefCell, collections::BTreeMap, collections::BTreeSet, rc::Rc};

type ModuleCache = Rc<RefCell<BTreeMap<String, RegistryKey>>>;
type Loading = Rc<RefCell<BTreeSet<String>>>;

/// Replace the absent standard loader with a bundle-backed, filesystem-free one.
pub(super) fn install_require(
    lua: &Lua,
    bundle: &SourceBundle,
    context: &ExecutionContext,
) -> Result<()> {
    let bundle = bundle.clone();
    let cache: ModuleCache = Rc::new(RefCell::new(BTreeMap::new()));
    let loading: Loading = Rc::new(RefCell::new(BTreeSet::new()));
    let require = lua
        .create_function(move |lua, name: String| {
            names::module(&name)
                .map_err(|error| mlua::Error::RuntimeError(error.message().to_owned()))?;
            if let Some(key) = cache.borrow().get(&name) {
                return lua.registry_value::<Value>(key);
            }
            let Some(source) = bundle.module(&name) else {
                return Err(mlua::Error::RuntimeError(format!(
                    "module {name:?} is not part of the captured bundle"
                )));
            };
            if !loading.borrow_mut().insert(name.clone()) {
                return Err(mlua::Error::RuntimeError(format!(
                    "module cycle at {name:?}"
                )));
            }
            let evaluated = lua
                .load(source.bytes().to_vec())
                .set_name(format!("@{}/lib/{name}.lua", bundle.command()))
                .eval::<Value>();
            loading.borrow_mut().remove(&name);
            let value = evaluated?;
            let value = if value.is_nil() {
                Value::Boolean(true)
            } else {
                value
            };
            let key = lua.create_registry_value(value.clone())?;
            cache.borrow_mut().insert(name, key);
            Ok(value)
        })
        .map_err(|error| error::map(context, error, "cannot install the module loader"))?;
    lua.globals()
        .set("require", require)
        .map_err(|error| error::map(context, error, "cannot install the module loader"))?;
    Ok(())
}
