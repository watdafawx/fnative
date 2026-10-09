// every export of the system's version.dll, forwarded to it: the game uses them as if this DLL were that one
const EXPORTS: [&str; 16] = [
    "GetFileVersionInfoA", "GetFileVersionInfoExA", "GetFileVersionInfoExW",
    "GetFileVersionInfoSizeA", "GetFileVersionInfoSizeExA", "GetFileVersionInfoSizeExW", "GetFileVersionInfoSizeW",
    "GetFileVersionInfoW", "VerFindFileA", "VerFindFileW", "VerInstallFileA", "VerInstallFileW", "VerLanguageNameA",
    "VerLanguageNameW", "VerQueryValueA", "VerQueryValueW",
];

fn main() {
    for e in EXPORTS {
        println!(r"cargo:rustc-cdylib-link-arg=/EXPORT:{e}=C:\Windows\System32\version.{e}");
    }
}
