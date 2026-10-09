-- the data stage gets the native table too
log(native and ("native-demo: data stage sees fse " .. native.version()) or "native-demo: no native table (plain game)")
