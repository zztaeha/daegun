#!/usr/bin/env python3
USAGE = """    release.py [--check] [--out DIR] [--target TRIPLE ...]

The C release, for each platform this machine builds for: an archive of the headers, the static library, the shared
one named by the C ABI's major as the README installs it, a relocatable daegun.pc, CMake's package files, and the
licenses, with SHA256SUMS and a BUILDINFO naming the commit and the tools that made them beside them in DIR
(target/release-archives by default, cleared of earlier archives first). Windows's DLL is versioned, beside its
import library, and carries its own VC runtime, needing only the UCRT that Windows 10 and later have. Android's
libdaegun.so is unversioned, as an app packs it, and every 64-bit Android library sits on 16 KB pages. Linux's
libraries need glibc 2.17 at most; musl's, iOS's, Mac Catalyst's, tvOS's and visionOS's are static alone. With both
macOS slices comes a universal macOS archive, and an XCFramework of static frameworks holds every slice whose
targets were built; every Apple archive and slice carries the privacy manifest Apple asks of an SDK. Each library is
held to the headers by abi-parity.py before it is packed, and no file packed may name the home directory. Nothing
builds under flags, a profile, a compiler, an SDK or a deployment target set in the environment or a cargo config
file; each is named. A release builds a commit as it stands, refusing a tree with changes, and needs the ABI and the
statuses recorded as released at the headers' version, saying how to record them. --check needs neither the commit
nor the records: it builds everything twice, in two target directories, into target/release-check, fails unless
every file matches byte for byte, and then builds the round trip from each archive as a user would with
release-use.py. --target builds those platforms alone. Nothing is uploaded. What a platform needs is checked before
anything builds: cargo-zigbuild 0.24 or newer and zig for Linux (older ones pass aarch64's --fix-cortex-a53-843419
to zig, which refuses it), the NDK r28 or newer at ANDROID_NDK_HOME or where Homebrew puts it for Android, and
llvm-rc and msvc-wine's runtime libraries in ~/opt/msvc for each Windows architecture."""

import gzip
import hashlib
import io
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
import zlib

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
INCLUDE = os.path.join(ROOT, "src", "api", "c", "include")
TARGETS = {
    "aarch64-apple-darwin": "macos",
    "x86_64-apple-darwin": "macos",
    "aarch64-apple-ios": "ios",
    "aarch64-apple-ios-sim": "ios",
    "x86_64-apple-ios": "ios",
    "aarch64-apple-ios-macabi": "catalyst",
    "x86_64-apple-ios-macabi": "catalyst",
    "aarch64-apple-tvos": "tvos",
    "aarch64-apple-tvos-sim": "tvos",
    "aarch64-apple-visionos": "visionos",
    "aarch64-apple-visionos-sim": "visionos",
    "aarch64-unknown-linux-gnu": "linux",
    "x86_64-unknown-linux-gnu": "linux",
    "aarch64-unknown-linux-musl": "musl",
    "x86_64-unknown-linux-musl": "musl",
    "aarch64-linux-android": "android",
    "x86_64-linux-android": "android",
    "armv7-linux-androideabi": "android",
    "aarch64-pc-windows-msvc": "windows",
    "x86_64-pc-windows-msvc": "windows",
}
# The platforms that take the static library alone: Apple's without a shared one, and musl, whose programs link
# statically.
STATIC_ONLY = {"ios", "catalyst", "tvos", "visionos", "musl"}
# Every entry's time, as a zip's earliest, so an archive depends on its bytes alone.
EPOCH = 315532800


def crate_version():
    # The crate's release, which names the archives and versions the DLL and the frameworks, as a release keeping the
    # ABI goes out under its own number; the ABI's own versions the .pc, CMake's files and the dylib.
    with open(os.path.join(ROOT, "Cargo.toml"), "rb") as f:
        return tomllib.load(f)["package"]["version"]


def abi_version():
    with open(os.path.join(INCLUDE, "daegun", "base.h"), encoding="utf-8") as f:
        hit = re.search(r"#define DAEGUN_ABI_VERSION 0x([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})u$", f.read(), re.M)
    return tuple(int(n, 16) for n in hit.groups())


def ndk():
    # The NDK, r28 or newer, at ANDROID_NDK_HOME or where Homebrew puts it: (its folder, its revision), or None.
    for root in (os.environ.get("ANDROID_NDK_HOME"), "/opt/homebrew/share/android-ndk", "/usr/local/share/android-ndk"):
        try:
            with open(os.path.join(root or "", "source.properties"), encoding="utf-8") as f:
                revision = re.search(r"Pkg\.Revision = (\S+)", f.read()).group(1)
        except (OSError, AttributeError):
            continue
        if int(revision.split(".")[0]) >= 28:
            return root, revision
    return None


def ndk_tool(name):
    root = ndk()[0]
    prebuilt = os.path.join(root, "toolchains", "llvm", "prebuilt")
    return os.path.join(prebuilt, sorted(os.listdir(prebuilt))[0], "bin", name)


def ndk_clang(target):
    # The NDK's clang for a Rust target at ANDROID_API, which names 32-bit Arm armv7a where Rust says armv7.
    return ndk_tool(f"{target.replace('armv7-', 'armv7a-', 1)}{ANDROID_API}-clang")


def pointer_size(target):
    return 4 if target.startswith(("armv7", "i686")) else 8


# The oldest Android a program here may run on, as Rust's own targets ask.
ANDROID_API = 21


def msvc_libs(target):
    msvc = os.path.expanduser("~/opt/msvc")
    arch = "arm64" if target.startswith("aarch64") else "x64"
    sdk = sorted(os.listdir(os.path.join(msvc, "kits", "10", "lib")))[-1]
    vc = sorted(os.listdir(os.path.join(msvc, "vc", "tools", "msvc")))[-1]
    paths = [os.path.join(msvc, "kits", "10", "lib", sdk, part, arch) for part in ("um", "ucrt")]
    paths.append(os.path.join(msvc, "vc", "tools", "msvc", vc, "lib", arch))
    return [arg for p in paths for arg in ("-C", f"link-arg=/LIBPATH:{p}")]


def missing(target):
    # What a platform needs that isn't here, with the command that installs it; None when nothing is.
    platform = TARGETS[target]
    installed = subprocess.run(["rustup", "target", "list", "--installed"], capture_output=True, text=True).stdout
    if target not in installed.split():
        return f"the {target} target (rustup target add {target})"
    if platform == "linux":
        version = subprocess.run(["cargo-zigbuild", "--version"], capture_output=True, text=True).stdout.split()
        if len(version) < 2 or tuple(int(n) for n in version[1].split(".")[:2]) < (0, 24):
            return "cargo-zigbuild 0.24 or newer (cargo install --locked cargo-zigbuild)"
    if platform == "android" and not ndk():
        return "the Android NDK r28 or newer, whose linker aligns to 16 KB pages (brew install --cask android-ndk)"
    if platform == "linux" and not llvm_rc():
        return "llvm-objcopy for the shared library's debug info (brew install llvm, or apt install llvm)"
    if platform == "windows":
        if not llvm_rc():
            return "llvm-rc for the DLL's version resource (brew install llvm, or apt install llvm)"
        arch = "arm64" if target.startswith("aarch64") else "x64"
        vc = os.path.expanduser("~/opt/msvc/vc/tools/msvc")
        versions = sorted(os.listdir(vc)) if os.path.isdir(vc) else []
        lib = os.path.join(vc, versions[-1], "lib", arch) if versions else ""
        # The static library's /MD runtime, and the static VC runtime the DLL links in.
        if not all(os.path.isfile(os.path.join(lib, n)) for n in ("msvcrt.lib", "libcmt.lib", "libvcruntime.lib")):
            return (f"msvc-wine's {arch} runtime libraries (vsdownload.py --accept-license --architecture {arch} "
                    "--dest ~/opt/msvc, then install.sh ~/opt/msvc; github.com/mstorsjo/msvc-wine)")
    return None


# What reaches a build from outside it and changes its bytes: flags, a profile, a compiler or its wrapper, incremental
# builds, an SDK or a deployment target.
OUTSIDE = re.compile(r"RUSTFLAGS|RUSTC(_WRAPPER|_WORKSPACE_WRAPPER)?|SDKROOT|\w+_DEPLOYMENT_TARGET"
                     r"|CARGO_(ENCODED_RUSTFLAGS|INCREMENTAL|BUILD_(RUSTFLAGS|RUSTC\w*|TARGET|INCREMENTAL)"
                     r"|PROFILE_\w+|TARGET_\w+_(RUSTFLAGS|LINKER)|UNSTABLE_\w+)")
BUILD_KEYS = ("rustflags", "rustc", "rustc-wrapper", "rustc-workspace-wrapper", "target", "incremental")


def outside():
    # Each such setting in the environment, or in a cargo config file in a folder above the crate or CARGO_HOME.
    found = [f"{name} in the environment" for name in sorted(os.environ) if OUTSIDE.fullmatch(name)]
    folders, d = [], ROOT
    while d not in folders:
        folders.append(d)
        d = os.path.dirname(d)
    configs = [os.path.join(f, ".cargo", n) for f in folders for n in ("config.toml", "config")]
    home = os.environ.get("CARGO_HOME") or os.path.expanduser("~/.cargo")
    configs += [os.path.join(home, n) for n in ("config.toml", "config")]
    for path in dict.fromkeys(configs):
        if not os.path.isfile(path):
            continue
        with open(path, "rb") as f:
            config = tomllib.load(f)
        keys = [f"build.{k}" for k in config.get("build", {}) if k in BUILD_KEYS]
        keys += [f"target.{t}.{k}" for t, v in config.get("target", {}).items() if isinstance(v, dict)
                 for k in v if k in ("rustflags", "linker")]
        keys += [k for k in ("profile", "env", "unstable", "include") if k in config]
        found += [f"{k} in {path}" for k in keys]
    return found


def remaps(target_dir):
    # std's panic locations name the local rust-src copy, and an MSVC object records its own path: both go back to
    # names any machine gives, std's as an official build spells them.
    run = lambda *a: subprocess.run(["rustc", *a], cwd=ROOT, capture_output=True, text=True, check=True).stdout
    commit = re.search(r"commit-hash: (\w+)", run("-vV")).group(1)
    sysroot = run("--print", "sysroot").strip()
    return [f"--remap-path-prefix={sysroot}/lib/rustlib/src/rust=/rustc/{commit}",
            f"--remap-path-prefix={target_dir}=target"]


def build(target, target_dir, abi, version):
    # One compile gives both libraries, link arguments reaching the shared one's link alone, but Windows's DLL takes
    # its own: (the static library's folder, its system libraries, the shared library's folder).
    platform, major = TARGETS[target], abi.split(".")[0]
    kinds = "staticlib" if platform in STATIC_ONLY | {"windows"} else "staticlib,cdylib"
    extra = ["--print", "native-static-libs"] + remaps(target_dir)
    triple = target
    if platform == "macos":
        # Debug map entries name objects by absolute path, which the dylib's UUID hashes. cargo-c's versions: the
        # ABI's, its major the oldest a program may load. The header keeps room for a bundler to rewrite the name.
        extra += ["-C", f"link-arg=-Wl,-install_name,@rpath/libdaegun.{major}.dylib",
                  "-C", f"link-arg=-Wl,-current_version,{abi}",
                  "-C", f"link-arg=-Wl,-compatibility_version,{major}",
                  "-C", "link-arg=-Wl,-headerpad_max_install_names",
                  "-C", f"link-arg=-Wl,-oso_prefix,{target_dir}"]
    elif platform == "linux":
        # Against glibc 2.17, Rust's floor: zig's default is its own release's, whose symbol versions a loader
        # older than it refuses. The build-id hashes the contents, so it names this build and moves no byte.
        triple = f"{target}.2.17"
        extra += ["-C", f"link-arg=-Wl,-soname,libdaegun.so.{major}", "-C", "link-arg=-Wl,--build-id=sha1",
                  "-C", "strip=none"]
    elif platform == "android":
        # Unversioned, as an app packs and loads lib*.so alone; the NDK's linker aligns to the 16 KB pages Google
        # Play asks of an app from Android 15 on.
        extra += ["-C", f"linker={ndk_clang(target)}",
                  "-C", "link-arg=-Wl,-soname,libdaegun.so", "-C", "link-arg=-Wl,--build-id=sha1"]
    cargo = ["cargo-zigbuild"] if platform == "linux" else ["cargo"]
    native = compile_(cargo, target, triple, kinds, extra, target_dir)
    built = os.path.join(target_dir, target, "release")
    if platform == "linux":
        # zig drops the symbol table with the debug info, which leaves a crash report and a profiler no names, so it
        # links whole and the debug info goes after.
        objcopy = os.path.join(os.path.dirname(shutil.which(llvm_rc())), "llvm-objcopy")
        subprocess.run([objcopy, "--strip-debug", os.path.join(built, "libdaegun.so")], check=True)
    return built, native, dll(target, target_dir, version) if platform == "windows" else built


def compile_(cargo, target, triple, kinds, extra, target_dir):
    argv = cargo + ["rustc", "--release", "--features", "capi", "--crate-type", kinds, "--target", triple, "--"]
    run = subprocess.run(argv + extra, cwd=ROOT, env={**os.environ, "CARGO_TARGET_DIR": target_dir},
                         capture_output=True, text=True)
    if run.returncode != 0:
        sys.exit(f"{target} does not build:\n{run.stderr[-3000:]}")
    native = re.search(r"native-static-libs: (.*)", run.stderr)
    return native.group(1).strip() if native else ""


def dll(target, target_dir, version):
    # Microsoft's hybrid CRT: the VC runtime linked in, the UCRT a DLL, so the DLL needs no redistributable, where the
    # static library stays /MD. No PDB ships, whose GUID, written into the DLL, hashes the build's paths.
    extra = remaps(target_dir) + [
        "-C", "target-feature=+crt-static", "-C", "linker=rust-lld", "-C", "link-arg=/Brepro",
        "-C", "link-arg=/DEBUG:NONE", "-C", "link-arg=/NODEFAULTLIB:libucrt.lib", "-C", "link-arg=/DEFAULTLIB:ucrt.lib",
        "-C", f"link-arg={resource(target_dir, version)}"] + msvc_libs(target)
    if target.startswith("x86_64"):
        # Shadow stacks, which Windows enforces in a process whose every module is marked; arm64 has none.
        extra += ["-C", "link-arg=/CETCOMPAT"]
    at = os.path.join(target_dir, "dll")
    compile_(["cargo"], target, target, "cdylib", extra, at)
    built = os.path.join(at, target, "release")
    readobj = os.path.join(os.path.dirname(shutil.which(llvm_rc())), "llvm-readobj")
    imports = subprocess.run([readobj, "--coff-imports", os.path.join(built, "daegun.dll")], capture_output=True,
                             text=True, check=True).stdout
    runtime = sorted(set(re.findall(r"Name: ((?:VCRUNTIME|MSVCP)\w*\.dll)", imports, re.I)))
    if runtime:
        sys.exit(f"{target}: daegun.dll needs {', '.join(runtime)}, which a machine without the VC++ redistributable "
                 "lacks")
    # It ships under the ABI's major, as 1.2's plain name would share one DLL between two plugins in a process, and
    # cargo's import library names daegun.dll, so one is made from its exports.
    major = abi_version()[0]
    exports = subprocess.run([readobj, "--coff-exports", os.path.join(built, "daegun.dll")], capture_output=True,
                             text=True, check=True).stdout
    definition = os.path.join(built, "daegun.def")
    with open(definition, "w", encoding="utf-8") as f:
        f.write(f"LIBRARY daegun-{major}.dll\nEXPORTS\n")
        f.writelines(f"{name}\n" for name in sorted(set(re.findall(r"^\s*Name: (\w+)$", exports, re.M))))
    machine = {"x86_64": "i386:x86-64", "aarch64": "arm64"}[target.split("-")[0]]
    subprocess.run([os.path.join(os.path.dirname(readobj), "llvm-dlltool"), "-m", machine, "-d", definition, "-l",
                    os.path.join(built, f"daegun-{major}.dll.lib")], check=True)
    return built


def resource(target_dir, version):
    # The DLL's version resource, which Windows Installer reads to decide a replacement; llvm-rc stamps no time, and
    # with nothing to preprocess needs no clang.
    numbers = ",".join(version.split(".") + ["0"])
    # Microsoft lists CompanyName as required; it and the copyright come from LICENSE, so neither drifts from it.
    with open(os.path.join(ROOT, "LICENSE"), encoding="utf-8") as f:
        line = re.search(r"^Copyright \(c\) \S+ (.+)$", f.read(), re.M)
    rc = "\n".join([
        "1 VERSIONINFO", f"FILEVERSION {numbers}", f"PRODUCTVERSION {numbers}", "FILEFLAGSMASK 0x3f", "FILEFLAGS 0",
        "FILEOS 0x40004", "FILETYPE 2", "FILESUBTYPE 0", "BEGIN", '  BLOCK "StringFileInfo"', "  BEGIN",
        '    BLOCK "040904b0"', "    BEGIN", f'      VALUE "CompanyName", "{line.group(1)}"',
        '      VALUE "FileDescription", "daegun: the C ABI"', f'      VALUE "FileVersion", "{version}"',
        '      VALUE "InternalName", "daegun"', f'      VALUE "LegalCopyright", "{line.group(0)}"',
        f'      VALUE "OriginalFilename", "daegun-{abi_version()[0]}.dll"', '      VALUE "ProductName", "daegun"',
        f'      VALUE "ProductVersion", "{version}"', "    END", "  END", '  BLOCK "VarFileInfo"', "  BEGIN",
        '    VALUE "Translation", 0x409, 1200', "  END", "END", ""])
    os.makedirs(target_dir, exist_ok=True)
    source, compiled = os.path.join(target_dir, "daegun.rc"), os.path.join(target_dir, "daegun.res")
    with open(source, "w", encoding="ascii") as f:
        f.write(rc)
    # Relative names, as llvm-rc reads a path starting /U as its /U option.
    subprocess.run([llvm_rc(), "-no-preprocess", "/FO", "daegun.res", "daegun.rc"], cwd=target_dir, check=True,
                   capture_output=True)
    return compiled


def llvm_version():
    # llvm-rc has no --version, so its version is that of llvm-readobj beside it.
    readobj = os.path.join(os.path.dirname(shutil.which(llvm_rc()) or ""), "llvm-readobj")
    ran = subprocess.run([readobj, "--version"], capture_output=True, text=True)
    return next((line.strip() for line in ran.stdout.split("\n") if "version" in line), "unknown")


def llvm_rc():
    found = [p for p in ("llvm-rc", "/opt/homebrew/opt/llvm/bin/llvm-rc", "/usr/local/opt/llvm/bin/llvm-rc")
             if shutil.which(p)]
    return found[0] if found else None


def pc(version, platform, native):
    # Windows's -ldaegun is the static daegun.lib, whose system libraries every link needs, so they go in Libs.
    system = " ".join(lib if lib.startswith("-") else f"-l{lib}" for lib in native_libs(platform, native))
    if platform == "windows":
        libs = [f"Libs: -L${{libdir}} -ldaegun {system}"]
    else:
        libs = ["Libs: -L${libdir} -ldaegun"] + ([f"Libs.private: {system}"] if system else [])
    return "\n".join([
        "prefix=${pcfiledir}/../..", "libdir=${prefix}/lib", "includedir=${prefix}/include", "",
        "Name: daegun", "Description: Text and shaping engine: the C ABI", f"Version: {version}",
        *libs, "Cflags: -I${includedir}", ""])


# What each platform's compiler driver links of itself, which a list naming it only repeats: Apple's libSystem, libc
# and libm, on musl gcc's unwinder for Rust's LLVM libunwind, on Android the NDK's libc, libdl and unwinder.
APPLE_DRIVER = {"-lSystem", "-lc", "-lm"}
DRIVER = {"linux": {"-lc"}, "musl": {"-lc", "-lunwind"}, "android": {"-lc", "-ldl", "-lunwind"},
          "macos": APPLE_DRIVER, "ios": APPLE_DRIVER, "catalyst": APPLE_DRIVER, "tvos": APPLE_DRIVER,
          "visionos": APPLE_DRIVER}


def native_libs(platform, native):
    # The system libraries the static library needs, as one item each: MSVC's names without .lib, and a framework
    # with the word before it. MSVC's default runtime is the compiler's to choose.
    words, out = native.split(), []
    while words:
        word = words.pop(0)
        if word in DRIVER.get(platform, ()):
            continue
        if word == "-framework" and words:
            out.append(f"-framework {words.pop(0)}")
        elif platform == "windows":
            if word.lower().endswith(".lib"):
                out.append(word[:-4])
        else:
            out.append(word)
    return out


def cmake(version, platform, major, native, pointer):
    # A package CMake's find_package reads with no pkg-config: daegun::daegun, the shared library where one ships,
    # and daegun::static; any version of the same major answers, as the ABI promises.
    libs = native_libs(platform, native)
    static = "lib/daegun.lib" if platform == "windows" else "lib/libdaegun.a"
    shared = {"macos": f"lib/libdaegun.{major}.dylib", "linux": f"lib/libdaegun.so.{major}",
              "android": "lib/libdaegun.so", "windows": f"bin/daegun-{major}.dll"}.get(platform)
    lines = ['get_filename_component(_daegun "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)',
             "if(NOT TARGET daegun::static)",
             "  add_library(daegun::static STATIC IMPORTED)",
             "  set_target_properties(daegun::static PROPERTIES",
             '    INTERFACE_INCLUDE_DIRECTORIES "${_daegun}/include"',
             f'    IMPORTED_LOCATION "${{_daegun}}/{static}"',
             f'    INTERFACE_LINK_LIBRARIES "{";".join(libs)}")']
    if shared:
        lines += ["  add_library(daegun::daegun SHARED IMPORTED)",
                  "  set_target_properties(daegun::daegun PROPERTIES",
                  '    INTERFACE_INCLUDE_DIRECTORIES "${_daegun}/include"',
                  f'    IMPORTED_LOCATION "${{_daegun}}/{shared}"']
        if platform == "windows":
            lines += [f'    IMPORTED_IMPLIB "${{_daegun}}/lib/daegun-{major}.dll.lib"',
                      "    INTERFACE_COMPILE_DEFINITIONS DAEGUN_DLL)"]
        elif platform == "linux":
            lines += [f"    IMPORTED_SONAME libdaegun.so.{major})"]
        elif platform == "android":
            lines += ["    IMPORTED_SONAME libdaegun.so)"]
        else:
            lines += [f"    IMPORTED_SONAME @rpath/libdaegun.{major}.dylib)"]
    else:
        # The static library where none ships shared, so one CMakeLists serves every platform.
        lines += ["  add_library(daegun::daegun INTERFACE IMPORTED)",
                  "  set_target_properties(daegun::daegun PROPERTIES INTERFACE_LINK_LIBRARIES daegun::static)"]
    # The headers' static assertions need C11, which cl takes only when asked; CMake 3.22 and on skip a C feature
    # in a project without C, where an older one would fail a C++ project.
    lines += ["  if(CMAKE_VERSION VERSION_GREATER_EQUAL 3.22)",
              "    set_property(TARGET daegun::static daegun::daegun APPEND",
              "      PROPERTY INTERFACE_COMPILE_FEATURES c_std_11)",
              "  endif()", "endif()", "unset(_daegun)", ""]
    # As CMake's SameMajorVersion file reads: a range only inside this major, and a program of another pointer size
    # than the archive's refused.
    config_version = "\n".join([
        f'set(PACKAGE_VERSION "{version}")',
        "if(PACKAGE_VERSION VERSION_LESS PACKAGE_FIND_VERSION)",
        "  set(PACKAGE_VERSION_COMPATIBLE FALSE)",
        "elseif(PACKAGE_FIND_VERSION_RANGE)",
        f'  if(PACKAGE_FIND_VERSION_MIN_MAJOR STREQUAL "{major}" AND',
        '     ((PACKAGE_FIND_VERSION_RANGE_MAX STREQUAL "INCLUDE"',
        f'       AND PACKAGE_FIND_VERSION_MAX_MAJOR STREQUAL "{major}"',
        "       AND PACKAGE_VERSION VERSION_LESS_EQUAL PACKAGE_FIND_VERSION_MAX)",
        '      OR (PACKAGE_FIND_VERSION_RANGE_MAX STREQUAL "EXCLUDE"',
        f'       AND PACKAGE_FIND_VERSION_MAX VERSION_LESS_EQUAL "{int(major) + 1}"',
        "       AND PACKAGE_VERSION VERSION_LESS PACKAGE_FIND_VERSION_MAX)))",
        "    set(PACKAGE_VERSION_COMPATIBLE TRUE)",
        "  else()",
        "    set(PACKAGE_VERSION_COMPATIBLE FALSE)",
        "  endif()",
        f'elseif(PACKAGE_FIND_VERSION_MAJOR STREQUAL "{major}")',
        "  set(PACKAGE_VERSION_COMPATIBLE TRUE)",
        "  if(PACKAGE_FIND_VERSION STREQUAL PACKAGE_VERSION)",
        "    set(PACKAGE_VERSION_EXACT TRUE)",
        "  endif()",
        "else()",
        "  set(PACKAGE_VERSION_COMPATIBLE FALSE)",
        "endif()",
        f'if(CMAKE_SIZEOF_VOID_P AND NOT CMAKE_SIZEOF_VOID_P STREQUAL "{pointer}")',
        f'  set(PACKAGE_VERSION "${{PACKAGE_VERSION}} ({pointer * 8}bit)")',
        "  set(PACKAGE_VERSION_UNSUITABLE TRUE)",
        "endif()", ""])
    return [("lib/cmake/daegun/daegun-config.cmake", "\n".join(lines).encode(), 0o644),
            ("lib/cmake/daegun/daegun-config-version.cmake", config_version.encode(), 0o644)]


def notices(at="share/doc/daegun/"):
    # The licenses of what the libraries carry, where cargo-c installs them so two archives in one prefix don't
    # clash: daegun's, CREDITS.txt's (the code and data daegun carries from others), and Rust's std's.
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True, check=True).stdout
    paths = [("LICENSE", os.path.join(ROOT, "LICENSE")), ("CREDITS.txt", os.path.join(ROOT, "CREDITS.txt")),
             ("LICENSE-rust-std.html", os.path.join(sysroot.strip(), "share", "doc", "rust", "COPYRIGHT-library.html"))]
    out = []
    for name, path in paths:
        with open(path, "rb") as f:
            out.append((f"{at}{name}", f.read(), 0o644))
    return out


def headers():
    out = []
    for dirpath, _, files in os.walk(INCLUDE):
        for name in files:
            if name.endswith(".h"):
                full = os.path.join(dirpath, name)
                with open(full, "rb") as f:
                    out.append((os.path.join("include", os.path.relpath(full, INCLUDE)), f.read(), 0o644))
    return out


def contents(target, built, native, shared, version, major):
    # (path in the archive, bytes or a symlink's target, mode). Apple's carry the privacy manifest a framework
    # does, for an app linking the library itself to merge into its own.
    platform = TARGETS[target]
    out = headers() + notices() + cmake(version, platform, major, native, pointer_size(target))
    if platform in {"macos", "ios", "catalyst", "tvos", "visionos"}:
        out.append(("share/daegun/PrivacyInfo.xcprivacy", plistlib.dumps(PRIVACY, sort_keys=True), 0o644))

    def read(name, at=built):
        with open(os.path.join(at, name), "rb") as f:
            return f.read()

    if platform == "windows":
        out += [("lib/daegun.lib", read("daegun.lib"), 0o644),
                (f"lib/daegun-{major}.dll.lib", read(f"daegun-{major}.dll.lib", shared), 0o644),
                (f"bin/daegun-{major}.dll", read("daegun.dll", shared), 0o755),
                ("lib/pkgconfig/daegun.pc", pc(version, platform, native).encode(), 0o644)]
        return out
    out.append(("lib/libdaegun.a", read("libdaegun.a"), 0o644))
    out.append(("lib/pkgconfig/daegun.pc", pc(version, platform, native).encode(), 0o644))
    if platform == "macos":
        out += [(f"lib/libdaegun.{major}.dylib", read("libdaegun.dylib"), 0o755),
                ("lib/libdaegun.dylib", f"libdaegun.{major}.dylib", 0o777)]
    elif platform == "linux":
        out += [(f"lib/libdaegun.so.{major}", read("libdaegun.so"), 0o755),
                ("lib/libdaegun.so", f"libdaegun.so.{major}", 0o777)]
    elif platform == "android":
        out.append(("lib/libdaegun.so", read("libdaegun.so"), 0o755))
    return out


# Swift and Objective-C import the headers as one module through this, in each framework of the XCFramework.
MODULEMAP = "framework module daegun {\n    umbrella header \"daegun.h\"\n    export *\n}\n"
# Rust's standard library reads its own binary's size with fstat to symbolize a panic's backtrace, which keeps fstat
# in any program linking the library, and Apple wants every SDK using fstat to say why: C617.1, a file in the app.
PRIVACY = {"NSPrivacyTracking": False, "NSPrivacyTrackingDomains": [], "NSPrivacyCollectedDataTypes": [],
           "NSPrivacyAccessedAPITypes": [{"NSPrivacyAccessedAPIType": "NSPrivacyAccessedAPICategoryFileTimestamp",
                                          "NSPrivacyAccessedAPITypeReasons": ["C617.1"]}]}


def minimum_os(lib):
    # The newest OS any object in the library asks for, as its load commands record it.
    out = subprocess.run(["otool", "-l", lib], capture_output=True, text=True, check=True).stdout
    found, command = set(), ""
    for line in out.splitlines():
        words = line.split()
        if words[:1] == ["cmd"]:
            command = words[1]
        elif len(words) == 2 and (command.startswith("LC_VERSION_MIN") and words[0] == "version"
                                  or command == "LC_BUILD_VERSION" and words[0] == "minos"):
            found.add(tuple(int(n) for n in words[1].split(".")))
    return ".".join(map(str, max(found)))


def catalyst_macos(version):
    # The macOS a Mac Catalyst version runs on, as Catalyst numbers its versions as iOS does, from 26 the two agreeing.
    # Its minors don't follow macOS's (14.3 is 11.1), so a minimum outside Apple's pairs here, which hold Rust's, stops.
    pairs = {"13.1": "10.15", "14.0": "11.0", "15.0": "12.0", "16.0": "13.0", "17.0": "14.0", "18.0": "15.0"}
    if int(version.split(".")[0]) >= 26:
        return version
    if version not in pairs:
        sys.exit(f"Mac Catalyst {version} has no macOS listed here; add Apple's pair to catalyst_macos")
    return pairs[version]


# Each framework of the XCFramework: its folder, the targets lipo'd into it, and its platform as Info.plist names it.
SLICES = [("macos", ("aarch64-apple-darwin", "x86_64-apple-darwin"), "MacOSX"),
          ("ios", ("aarch64-apple-ios",), "iPhoneOS"),
          ("ios-simulator", ("aarch64-apple-ios-sim", "x86_64-apple-ios"), "iPhoneSimulator"),
          ("catalyst", ("aarch64-apple-ios-macabi", "x86_64-apple-ios-macabi"), "MacOSX"),
          ("tvos", ("aarch64-apple-tvos",), "AppleTVOS"),
          ("tvos-simulator", ("aarch64-apple-tvos-sim",), "AppleTVSimulator"),
          ("visionos", ("aarch64-apple-visionos",), "XROS"),
          ("visionos-simulator", ("aarch64-apple-visionos-sim",), "XRSimulator")]


def framework(work, slice_name, lib, platform, version):
    # A static framework around the library, as Apple has a static library carry its privacy manifest: versioned
    # for macOS and Mac Catalyst, flat for the rest, the manifest beside Info.plist.
    root = os.path.join(work, slice_name, "daegun.framework")
    mac = platform == "MacOSX"
    base = os.path.join(root, "Versions", "A") if mac else root
    resources = os.path.join(base, "Resources") if mac else base
    # Flat, as a framework's <daegun/x.h> reads Headers/x.h.
    os.makedirs(os.path.join(base, "Headers"))
    for path, data, _ in headers():
        with open(os.path.join(base, "Headers", os.path.basename(path)), "wb") as f:
            f.write(data)
    os.makedirs(os.path.join(base, "Modules"))
    os.makedirs(resources, exist_ok=True)
    with open(os.path.join(base, "Modules", "module.modulemap"), "w", encoding="utf-8") as f:
        f.write(MODULEMAP)
    shutil.copyfile(lib, os.path.join(base, "daegun"))
    info = {"CFBundleDevelopmentRegion": "en", "CFBundleExecutable": "daegun",
            "CFBundleIdentifier": "dev.zztaeha.daegun", "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleName": "daegun", "CFBundlePackageType": "FMWK",
            "CFBundleShortVersionString": version, "CFBundleSupportedPlatforms": [platform], "CFBundleVersion": version,
            "LSMinimumSystemVersion" if mac else "MinimumOSVersion": minimum_os(lib)}
    if slice_name == "catalyst":
        info["LSMinimumSystemVersion"] = catalyst_macos(info["LSMinimumSystemVersion"])
    for name, value in (("Info.plist", info), ("PrivacyInfo.xcprivacy", PRIVACY)):
        with open(os.path.join(resources, name), "wb") as f:
            f.write(plistlib.dumps(value, sort_keys=True))
    if mac:
        os.symlink("A", os.path.join(root, "Versions", "Current"))
        for name in ("daegun", "Headers", "Modules", "Resources"):
            os.symlink(f"Versions/Current/{name}", os.path.join(root, name))
    return root


def apple(built_at, target_dir, abi, version, major):
    # The two macOS slices as one universal library, and an XCFramework of every slice fully built: (archive name,
    # the folder inside, files, whether a zip). SwiftPM wants the XCFramework at the zip's root: that zip has no folder.
    out = []
    work = os.path.join(target_dir, "apple")
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)

    def lipo(name, slices, into):
        fat = os.path.join(work, into)
        subprocess.run(["lipo", "-create", *[os.path.join(b, name) for b, _ in slices], "-output", fat], check=True)
        with open(fat, "rb") as f:
            return fat, f.read()

    mac = [built_at[t] for t in ("aarch64-apple-darwin", "x86_64-apple-darwin") if t in built_at]
    if len(mac) == 2:
        static_bytes, dylib = lipo("libdaegun.a", mac, "libdaegun.a")[1], lipo("libdaegun.dylib", mac, "dylib")[1]
        native = mac[0][1]
        stem = f"daegun-{version}-universal-apple-darwin"
        out.append((f"{stem}.tar.gz", stem, headers() + notices() + cmake(abi, "macos", major, native, 8) + [
            ("lib/libdaegun.a", static_bytes, 0o644), (f"lib/libdaegun.{major}.dylib", dylib, 0o755),
            ("lib/libdaegun.dylib", f"libdaegun.{major}.dylib", 0o777),
            ("lib/pkgconfig/daegun.pc", pc(abi, "macos", native).encode(), 0o644),
            ("share/daegun/PrivacyInfo.xcprivacy", plistlib.dumps(PRIVACY, sort_keys=True), 0o644)], False))
    frameworks = [framework(work, name, lipo("libdaegun.a", [built_at[t] for t in targets], f"{name}.a")[0],
                            platform, version)
                  for name, targets, platform in SLICES if all(t in built_at for t in targets)]
    if not frameworks:
        return out
    xcf = os.path.join(work, "daegun.xcframework")
    argv = ["xcodebuild", "-create-xcframework", "-output", xcf]
    for path in frameworks:
        argv += ["-framework", path]
    made = subprocess.run(argv, capture_output=True, text=True)
    if made.returncode != 0:
        sys.exit(f"the XCFramework does not build:\n{made.stdout[-2000:]}{made.stderr[-2000:]}")
    # xcodebuild lists the frameworks in no set order, so the same ones could make another file: sorted, they can't.
    with open(os.path.join(xcf, "Info.plist"), "rb") as f:
        info = plistlib.load(f)
    info["AvailableLibraries"].sort(key=lambda library: library["LibraryIdentifier"])
    with open(os.path.join(xcf, "Info.plist"), "wb") as f:
        f.write(plistlib.dumps(info, sort_keys=True))
    files = []
    for dirpath, dirs, names in os.walk(xcf):
        for name in dirs + names:
            full = os.path.join(dirpath, name)
            path = os.path.join("daegun.xcframework", os.path.relpath(full, xcf))
            if os.path.islink(full):
                files.append((path, os.readlink(full), 0o777))
            elif os.path.isfile(full):
                with open(full, "rb") as f:
                    files.append((path, f.read(), 0o644))
    out.append((f"daegun-{version}-xcframework.zip", "", files + notices(""), True))
    return out


def archive(stem, files, windows):
    files = sorted((f"{stem}/{path}" if stem else path, data, mode) for path, data, mode in files)
    buffer = io.BytesIO()
    if windows:
        with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for path, data, mode in files:
                info = zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0))
                info.external_attr = ((0o120000 if isinstance(data, str) else 0o100000) | mode) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                z.writestr(info, data)
        return f"{stem}.zip", buffer.getvalue()
    with gzip.GzipFile(filename="", mode="wb", fileobj=buffer, mtime=EPOCH, compresslevel=9) as gz:
        with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
            dirs = set()
            for path, _, _ in files:
                parts = path.split("/")[:-1]
                dirs.update("/".join(parts[:i]) for i in range(1, len(parts) + 1))
            for d in sorted(dirs):
                info = tarfile.TarInfo(d)
                info.type, info.mode, info.mtime = tarfile.DIRTYPE, 0o755, EPOCH
                tar.addfile(info)
            for path, data, mode in files:
                info = tarfile.TarInfo(path)
                info.mtime, info.mode = EPOCH, mode & 0o755
                if isinstance(data, str):
                    info.type, info.linkname = tarfile.SYMTYPE, data
                    tar.addfile(info)
                else:
                    info.size = len(data)
                    tar.addfile(info, io.BytesIO(data))
    return f"{stem}.tar.gz", buffer.getvalue()


def digest(files):
    # What an archive holds, apart from how it was compressed: each file's hash, mode and path.
    lines = [f"{hashlib.sha256(d if isinstance(d, bytes) else d.encode()).hexdigest()} {m:o} {p}" for p, d, m in files]
    return hashlib.sha256("\n".join(sorted(lines)).encode()).hexdigest()


def source():
    # The commit built and whether the tree differs from it; GIT_OPTIONAL_LOCKS keeps status from rewriting the
    # index. (None, False) outside a repository.
    git = lambda *a: subprocess.run(["git", *a], cwd=ROOT, env={**os.environ, "GIT_OPTIONAL_LOCKS": "0"},
                                    capture_output=True, text=True)
    head = git("rev-parse", "HEAD")
    if head.returncode != 0:
        return None, False
    return head.stdout.strip(), bool(git("status", "--porcelain").stdout.strip())


def buildinfo(targets, contents_of):
    # The commit, the tools whose versions the bytes depend on, and each archive's contents apart from its
    # compression, as another zlib compresses the same files to other bytes.
    def tool(argv, cwd=None):
        ran = subprocess.run(argv, capture_output=True, text=True, cwd=cwd)
        return ran.stdout + ran.stderr

    platforms, rustc = {TARGETS[t] for t in targets}, tool(["rustc", "-vV"])
    commit, changed = source()
    lines = [f"source {commit or 'outside a repository'}{' with changes' if changed else ''}",
             f"rustc {re.search(r'release: (\S+)', rustc).group(1)} {re.search(r'commit-hash: (\w+)', rustc).group(1)}",
             f"python {sys.version.split()[0]}, zlib {zlib.ZLIB_RUNTIME_VERSION}"]
    if platforms & {"macos", "ios", "catalyst", "tvos", "visionos"}:
        lines.append(f"ld {re.search(r'PROJECT:(\S+)', tool(['ld', '-v'])).group(1)}")
        xcode = re.findall(r"(?:Xcode|Build version) (\S+)", tool(["xcodebuild", "-version"]))
        lines.append(f"xcode {' '.join(xcode)}")
        for sdk in ("macosx", "iphoneos", "iphonesimulator", "appletvos", "appletvsimulator", "xros", "xrsimulator"):
            lines.append(f"sdk {sdk} {tool(['xcrun', '--sdk', sdk, '--show-sdk-version']).strip()}")
    if "linux" in platforms:
        # zig's clang, whose LLVM each Linux library names in its .comment section. Asked from a scratch folder, as
        # `zig cc --version` leaves an empty a.o wherever it runs.
        with tempfile.TemporaryDirectory() as scratch:
            zig_cc = tool(["zig", "cc", "--version"], scratch).split("\n")[0].strip()
        lines += [f"zig {tool(['zig', 'version']).strip()}", tool(["cargo-zigbuild", "--version"]).strip(),
                  f"zig cc: {zig_cc}"]
    if "android" in platforms:
        lines.append(f"ndk {ndk()[1]}")
    if "windows" in platforms:
        msvc = os.path.expanduser("~/opt/msvc")
        lines += [f"windows sdk {sorted(os.listdir(os.path.join(msvc, 'kits', '10', 'lib')))[-1]}",
                  f"msvc {sorted(os.listdir(os.path.join(msvc, 'vc', 'tools', 'msvc')))[-1]}",
                  f"llvm-rc: {llvm_version()}"]
    lines += [f"contents {name} {d}" for name, d in sorted(contents_of)]
    return "\n".join(lines) + "\n"


def unrecorded(version):
    # What a release needs recorded first, as abi-parity.py and c-statuses.py hold later versions to it, each with the
    # command that records it.
    tools = os.path.join(ROOT, "scripts", "tools", "c-api")
    with open(os.path.join(tools, "abi-released.txt"), encoding="utf-8") as f:
        recorded = f.readline().split()[1:]
    need = []
    if recorded != [version]:
        need.append(f"the ABI as released, recorded at {' '.join(recorded) or 'none'} "
                    "(python3 -I scripts/tools/c-api/abi-parity.py --record)")
    if not os.path.isfile(os.path.join(tools, "c-statuses-released.txt")):
        need.append("the statuses as released "
                    "(cp scripts/tools/c-api/c-statuses.txt scripts/tools/c-api/c-statuses-released.txt)")
    return need


def release(out_dir, target_dir, targets):
    major, minor, patch = abi_version()
    abi, version = f"{major}.{minor}.{patch}", crate_version()
    os.makedirs(out_dir, exist_ok=True)
    # Each run writes the whole set, so an archive an earlier run left can't sit beside sums that don't list it.
    for name in os.listdir(out_dir):
        if name.startswith("daegun-") or name in ("SHA256SUMS", "BUILDINFO"):
            os.remove(os.path.join(out_dir, name))
    sums, contents_of, built_at = [], [], {}
    for target in targets:
        print(f"  {target} ...", flush=True)
        built, native, shared = build(target, target_dir, abi, version)
        built_at[target] = (built, native)
        files = contents(target, built, native, shared, abi, major)
        libraries = [os.path.join(at, n) for at, n in ((built, "libdaegun.a"), (built, "daegun.lib"),
                                                        (shared, f"daegun-{abi_version()[0]}.dll.lib"),
                                                        (shared, "libdaegun.so"))
                     if os.path.isfile(os.path.join(at, n))]
        if TARGETS[target] == "macos":
            libraries.append(os.path.join(built, "libdaegun.dylib"))
        check = subprocess.run([sys.executable, "-I", os.path.join(ROOT, "scripts", "tools", "c-api", "abi-parity.py")]
                               + libraries, capture_output=True, text=True)
        if check.returncode != 0:
            sys.exit(f"{target}'s libraries differ from the headers:\n{check.stdout[-3000:]}")
        home = os.path.expanduser("~").encode()
        leaks = [path for path, data, _ in files if isinstance(data, bytes) and home in data]
        if leaks:
            sys.exit(f"{target}: these name a path on this machine: {', '.join(leaks)}")
        name, data = archive(f"daegun-{version}-{target}", files, TARGETS[target] == "windows")
        with open(os.path.join(out_dir, name), "wb") as f:
            f.write(data)
        sums.append(f"{hashlib.sha256(data).hexdigest()}  {name}")
        contents_of.append((name, digest(files)))
    for name, stem, files, windows in apple(built_at, target_dir, abi, version, major):
        _, data = archive(stem, files, windows)
        with open(os.path.join(out_dir, name), "wb") as f:
            f.write(data)
        sums.append(f"{hashlib.sha256(data).hexdigest()}  {name}")
        contents_of.append((name, digest(files)))
    with open(os.path.join(out_dir, "BUILDINFO"), "w", encoding="utf-8") as f:
        f.write(buildinfo(targets, contents_of))
    with open(os.path.join(out_dir, "BUILDINFO"), "rb") as f:
        sums.append(f"{hashlib.sha256(f.read()).hexdigest()}  BUILDINFO")
    with open(os.path.join(out_dir, "SHA256SUMS"), "w", encoding="utf-8") as f:
        f.write("\n".join(sorted(sums, key=lambda s: s.split()[1])) + "\n")
    return sorted(s.split()[1] for s in sums) + ["SHA256SUMS"]


def main(argv):
    check = "--check" in argv
    out = os.path.join(ROOT, "target", "release-check" if check else "release-archives")
    if "--out" in argv:
        at = argv.index("--out")
        out = os.path.abspath(argv[at + 1])
        argv = argv[:at] + argv[at + 2:]
    targets = []
    while "--target" in argv:
        at = argv.index("--target")
        targets.append(argv[at + 1] if at + 1 < len(argv) else "")
        argv = argv[:at] + argv[at + 2:]
    if argv not in ([], ["--check"]) or any(t not in TARGETS for t in targets):
        print(USAGE)
        return 0 if argv in (["-h"], ["--help"]) else 1
    targets = targets or list(TARGETS)
    major, minor, patch = abi_version()
    needed = [m for m in map(missing, targets) if m] + ([] if check else unrecorded(f"{major}.{minor}.{patch}"))
    if needed:
        print("missing, so nothing was built:")
        print("\n".join(f"  {m}" for m in needed))
        return 1
    if found := outside():
        print("set outside the build, which would change what it makes, so nothing was built:")
        print("\n".join(f"  {m}" for m in found))
        return 1
    commit, changed = source()
    if not check and (commit is None or changed):
        print("a release builds a commit as it stands, and the tree " + ("isn't a repository" if commit is None
              else f"has changes against {commit[:12]}") + ", so nothing was built (commit them first)")
        return 1
    print(f"{'check' if check else 'release'} into {out}")
    written = release(out, os.path.join(ROOT, "target", "release-build"), targets)
    if argv == ["--check"]:
        again = out + "-again"
        print(f"again into {again}, from another target directory")
        release(again, os.path.join(ROOT, "target", "release-build-again"), targets)
        def read(path):
            with open(path, "rb") as f:
                return f.read()

        differ = [n for n in written if read(os.path.join(out, n)) != read(os.path.join(again, n))]
        if differ:
            print("issues: these differ between the two builds")
            print("\n".join(f"  {n}" for n in differ))
            return 1
        print(f"{len(written)} files the same from both builds")
        use = os.path.join(ROOT, "scripts", "tools", "release", "release-use.py")
        used = subprocess.run([sys.executable, "-I", use, out])
        if used.returncode != 0:
            return 1
    with open(os.path.join(out, "SHA256SUMS"), encoding="utf-8") as f:
        print(f.read(), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
