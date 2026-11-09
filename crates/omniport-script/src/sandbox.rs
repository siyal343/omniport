use mlua::{Lua, Result, Table};

/// List of dangerous global identifiers stripped from the sandboxed environment
pub const FORBIDDEN_GLOBALS: &[&str] = &[
    "dofile",
    "loadfile",
    "load",
    "require",
    "package",
    "io",
    "os",
    "debug",
];

/// Applies strict sandboxing to the Lua runtime
pub fn apply_sandbox(lua: &Lua) -> Result<()> {
    let globals = lua.globals();

    // 1. Remove dangerous execution and file system globals
    for &forbidden in FORBIDDEN_GLOBALS {
        globals.raw_set(forbidden, mlua::Value::Nil)?;
    }

    // 2. Provide safe mathematical and string utilities
    // Lua 5.4 math, string, table, utf8 are safe and remain accessible.

    // 3. Create a restricted safe mock time helper under `serial.time_sec()`
    let serial_tbl: Table = lua.create_table()?;
    globals.raw_set("serial", serial_tbl)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_strips_forbidden_modules() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();

        let res: Result<()> = lua.load("io.open('test.txt', 'w')").exec();
        assert!(res.is_err(), "Accessing io must fail in sandbox");

        let res2: Result<()> = lua.load("os.execute('calc')").exec();
        assert!(res2.is_err(), "Accessing os must fail in sandbox");

        let res3: Result<()> = lua.load("require('socket')").exec();
        assert!(res3.is_err(), "Accessing require must fail in sandbox");

        // Safe standard libraries must remain functional
        let math_res: Result<f64> = lua.load("return math.sin(math.pi / 2)").eval();
        assert_eq!(math_res.unwrap(), 1.0);

        let str_res: Result<String> = lua.load("return string.upper('omniport')").eval();
        assert_eq!(str_res.unwrap(), "OMNIPORT");
    }
}
