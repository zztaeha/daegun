#!/usr/bin/env python3
USAGE = """    release-use.py [DIR]

Builds the C round trip from each archive in DIR (target/release-check by default) as someone installing it would,
and runs it wherever this machine can. A macOS archive builds through pkg-config, shared and static, and CMake's two
targets, for each architecture it holds, and the universal one's version file answers the versions and ranges
find_package asks; each XCFramework slice builds a program importing the daegun module, the macOS and Mac Catalyst
slices running it and the round trip, and a SwiftPM package taking the XCFramework's zip as a binary target, as an
app downloads it, builds and runs; the other Apple archives link the round trip, Mac Catalyst's running it. Linux's
run in a GCC image and musl's in an Alpine one, plain and -static, through pkg-config and CMake; an architecture
docker can't run here links with zig instead. Android's link with the NDK through pkg-config, CMake and the README's
line, a 64-bit one's every segment on 16 KB pages, and an app's shared library hides daegun's calls with the
README's flag. Windows's build with cl under Wine, against the static library and its DLL, directly and through
CMake and Ninja, which ask for C11 only through daegun's targets, and the x64 ones run, the DLL also in a /MT
program with Wine's VC runtime turned off. Every CMake project here leaves the C standard to those targets.
release.py --check runs this once its two builds match."""

import importlib.util
import os
import plistlib
import re
import shlex
import shutil
import subprocess
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
# release.py's own readers of the NDK and of a library's minimum OS, loaded by path as -I leaves its folder off.
_spec = importlib.util.spec_from_file_location("release", os.path.join(os.path.dirname(__file__), "release.py"))
release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release)
WORK = os.path.join(ROOT, "target", "release-use")
ROUNDTRIP = "src/api/c/tests/roundtrip.c"
FONT = "assets/test-fonts/inter/InterVariable.ttf"
# The NDK's name for each Android architecture Rust names.
ANDROID_ABI = {"aarch64": "arm64-v8a", "x86_64": "x86_64", "armv7": "armeabi-v7a"}
# Pinned images, with what a user of each builds with.
IMAGES = {"gnu": ("daegun-use-gnu:15.3.0", "FROM gcc:15.3.0\nRUN apt-get update && apt-get install -y "
                  "--no-install-recommends cmake && rm -rf /var/lib/apt/lists/*\n"),
          "musl": ("daegun-use-musl:3.22", "FROM alpine:3.22\nRUN apk add --no-cache build-base cmake pkgconf\n")}
CMAKE = """cmake_minimum_required(VERSION 3.19)
project(use C)
find_package(daegun 2.0 REQUIRED)
foreach(kind static daegun)
  add_executable(rt-${kind} ${ROUNDTRIP})
  target_link_libraries(rt-${kind} PRIVATE daegun::${kind})
endforeach()
"""
VERSIONS = """cmake_minimum_required(VERSION 3.19)
project(versions NONE)
if(DEFINED BITS)
  set(CMAKE_SIZEOF_VOID_P ${BITS})
endif()
find_package(daegun ${SPEC} QUIET)
message(STATUS "found ${daegun_FOUND}")
"""
# Each request and whether 2.x answers it: a range only inside the major, and a 32-bit program refused.
REQUESTS = [("2.0", "8", True), ("2.0...<3", "8", True), ("2.0...2.9", "8", True), ("1.0", "8", False),
            ("3.0", "8", False), ("2.0...3", "8", False), ("1.0...<3", "8", False), ("2.0", "4", False)]
SWIFT_PACKAGE = """// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "use",
    platforms: [.macOS(.v11)],
    targets: [
        .binaryTarget(name: "daegun", path: "ZIP"),
        .executableTarget(name: "use", dependencies: ["daegun"]),
    ]
)
"""
SWIFT_MAIN = """import daegun

let path = daegun_path_new()
daegun_path_free(path)
precondition(daegun_abi_version() == DAEGUN_ABI_VERSION, "another version")
var tag: daegun_tag = 0
let status = "wght".withCString { daegun_tag_from_string($0, 4, &tag) }
precondition(status == DAEGUN_OK && tag == 0x77676874, "no tag")
precondition(DAEGUN_PDF_ITALIC == UInt32(0x40), "a flag of another type")
let rasterizer = daegun_rasterizer_new()
let square = daegun_path_new()
precondition(daegun_path_move_to(square, 0, 0) == DAEGUN_OK && daegun_path_line_to(square, 2, 0) == DAEGUN_OK
    && daegun_path_line_to(square, 2, 2) == DAEGUN_OK && daegun_path_line_to(square, 0, 2) == DAEGUN_OK
    && daegun_path_close(square) == DAEGUN_OK, "no square")
var image = daegun_image()
precondition(daegun_rasterizer_path(rasterizer, square, DAEGUN_FILL_NONZERO, nil, &image) == DAEGUN_OK
    && image.width == 2 && image.pixels_len == 4 && image.pixels?[3] == 255, "no image")
daegun_path_free(square)
daegun_rasterizer_free(rasterizer)
"""
USE_M = """@import daegun;
int main(void) {
    daegun_path *p = daegun_path_new();
    int ok = p != 0 && DAEGUN_ABI_COMPATIBLE(daegun_abi_version());
    daegun_path_free(p);
    return !ok;
}
"""
# Each Apple target's SDK and clang target, and the XCFramework's platforms and variants as Info.plist names them.
APPLE = {"aarch64-apple-ios": ("iphoneos", "arm64-apple-ios"),
         "aarch64-apple-ios-sim": ("iphonesimulator", "arm64-apple-ios-simulator"),
         "x86_64-apple-ios": ("iphonesimulator", "x86_64-apple-ios-simulator"),
         "aarch64-apple-ios-macabi": ("macosx", "arm64-apple-ios-macabi"),
         "x86_64-apple-ios-macabi": ("macosx", "x86_64-apple-ios-macabi"),
         "aarch64-apple-tvos": ("appletvos", "arm64-apple-tvos"),
         "aarch64-apple-tvos-sim": ("appletvsimulator", "arm64-apple-tvos-simulator"),
         "aarch64-apple-visionos": ("xros", "arm64-apple-xros"),
         "aarch64-apple-visionos-sim": ("xrsimulator", "arm64-apple-xros-simulator")}
SLICES = {("macos", None): ("macosx", "macos"), ("ios", None): ("iphoneos", "ios"),
          ("ios", "simulator"): ("iphonesimulator", "ios-simulator"),
          ("ios", "maccatalyst"): ("macosx", "ios-macabi"), ("tvos", None): ("appletvos", "tvos"),
          ("tvos", "simulator"): ("appletvsimulator", "tvos-simulator"), ("xros", None): ("xros", "xros"),
          ("xros", "simulator"): ("xrsimulator", "xros-simulator")}


class Failed(Exception):
    pass


def at_minimum(triple, lib):
    # The clang target at the oldest OS the library's objects ask for, as an app's deployment target is at least
    # that; with no version clang falls back to one so old that its startup code is no longer in the SDK.
    parts = triple.split("-")
    parts[2] += release.minimum_os(lib)
    return "-".join(parts)


def run(argv, cwd=ROOT, env=None, what=None):
    done = subprocess.run(argv, cwd=cwd, env={**os.environ, "DAEGUN_ROOT": ROOT, **(env or {})},
                          capture_output=True, text=True)
    if done.returncode != 0:
        raise Failed(f"{what or argv[0]} failed:\n{(done.stdout + done.stderr)[-2000:]}")
    return done.stdout


def pkg(prefix, *args):
    # The flags pkg-config gives, reading this prefix's daegun.pc alone.
    return shlex.split(run(["pkg-config", *args, "daegun"], env={"PKG_CONFIG_LIBDIR": f"{prefix}/lib/pkgconfig"}))


def readme(pattern):
    # The README's words for a link, as a user copies them.
    with open(os.path.join(ROOT, "README.md"), encoding="utf-8") as f:
        hit = re.search(pattern, f.read(), re.M)
    if not hit:
        raise Failed(f"the README has no line matching {pattern}")
    return hit.group(1).split()


def static_copy(prefix, at):
    # The prefix without its shared library, as a static install is, so -ldaegun finds the archive.
    shutil.copytree(prefix, at, symlinks=True)
    for name in os.listdir(os.path.join(at, "lib")):
        if name.startswith("libdaegun.so") or name.endswith(".dylib"):
            os.remove(os.path.join(at, "lib", name))
    return at


def cmake_project(work):
    folder = os.path.join(work, "cmake")
    os.makedirs(folder, exist_ok=True)
    with open(os.path.join(folder, "CMakeLists.txt"), "w", encoding="utf-8") as f:
        f.write(CMAKE)
    return folder


def macos(prefix, work, archs):
    # An app's bundler rewrites the install name, which takes the room the link left in the header.
    renamed = os.path.join(work, "renamed.dylib")
    shutil.copy(os.path.join(prefix, "lib", "libdaegun.2.dylib"), renamed)
    run(["install_name_tool", "-id", f"@rpath/{'x' * 200}/libdaegun.2.dylib", renamed], what="a long install name")
    done = []
    for arch in archs:
        out = os.path.join(work, arch)
        os.makedirs(out)
        shared = os.path.join(out, "rt-shared")
        run(["cc", "-arch", arch, "-std=c11", "-Wall", "-Wextra", "-Werror", ROUNDTRIP,
             *pkg(prefix, "--cflags", "--libs"), f"-Wl,-rpath,{prefix}/lib", "-o", shared])
        if "libdaegun.2.dylib" not in run(["otool", "-L", shared]):
            raise Failed(f"{arch}: the shared round trip doesn't load libdaegun.2.dylib")
        run([shared, FONT], what=f"{arch} round trip, shared")
        alone = static_copy(prefix, os.path.join(out, "static"))
        static = os.path.join(out, "rt-static")
        run(["cc", "-arch", arch, "-std=c11", "-Wall", "-Wextra", "-Werror", ROUNDTRIP,
             *pkg(alone, "--static", "--cflags", "--libs"), "-o", static])
        if "libdaegun" in run(["otool", "-L", static]):
            raise Failed(f"{arch}: the static round trip loads a libdaegun")
        run([static, FONT], what=f"{arch} round trip, static")
        build = os.path.join(out, "cmake-build")
        run(["cmake", "-S", cmake_project(work), "-B", build, f"-DCMAKE_PREFIX_PATH={prefix}",
             f"-DCMAKE_OSX_ARCHITECTURES={arch}", f"-DROUNDTRIP={os.path.join(ROOT, ROUNDTRIP)}"])
        run(["cmake", "--build", build])
        for kind in ("static", "daegun"):
            run([os.path.join(build, f"rt-{kind}"), FONT], what=f"{arch} round trip, CMake daegun::{kind}")
        done.append(arch)
    return (f"{' and '.join(done)}: pkg-config shared and static, CMake daegun::daegun and daegun::static ran, "
            "and the dylib took a 200-character install name")


def versions(prefix, work):
    folder = os.path.join(work, "versions")
    os.makedirs(folder)
    with open(os.path.join(folder, "CMakeLists.txt"), "w", encoding="utf-8") as f:
        f.write(VERSIONS)
    for n, (spec, bits, answers) in enumerate(REQUESTS):
        build = os.path.join(work, f"versions-{n}")
        said = run(["cmake", "-S", folder, "-B", build, f"-DSPEC={spec}", f"-DBITS={bits}",
                    f"-DCMAKE_PREFIX_PATH={prefix}"])
        if ("found 1" in said) != answers:
            raise Failed(f"find_package(daegun {spec}) in a {int(bits) * 8}-bit project answered {not answers}")
    return f"find_package answers {len(REQUESTS)} requests as CMake's SameMajorVersion does"


def apple_static(target, prefix, work):
    sdk, triple = APPLE[target]
    triple = at_minimum(triple, os.path.join(prefix, "lib", "libdaegun.a"))
    out = os.path.join(work, "rt")
    run(["xcrun", "--sdk", sdk, "clang", "-target", triple, "-std=c11", "-Wall", "-Wextra", "-Werror", ROUNDTRIP,
         *pkg(prefix, "--static", "--cflags", "--libs"), "-o", out])
    if not triple.endswith("macabi"):
        return f"the round trip links for {triple} through pkg-config"
    run([out, FONT], what=f"{triple} round trip")
    return f"the round trip links for {triple} through pkg-config and runs"


def xcframework(xcf, archive, work):
    with open(os.path.join(xcf, "Info.plist"), "rb") as f:
        libraries = plistlib.load(f)["AvailableLibraries"]
    with open(os.path.join(work, "use.m"), "w", encoding="utf-8") as f:
        f.write(USE_M)
    ran, linked = [], []
    for library in libraries:
        sdk, os_part = SLICES[(library["SupportedPlatform"], library.get("SupportedPlatformVariant"))]
        at = os.path.join(xcf, library["LibraryIdentifier"])
        for arch in library["SupportedArchitectures"]:
            binary = os.path.join(at, "daegun.framework", "daegun")
            triple = at_minimum(f"{arch}-apple-{os_part}", binary)
            out = os.path.join(work, f"{library['LibraryIdentifier']}-{arch}")
            clang = ["xcrun", "--sdk", sdk, "clang", "-target", triple, "-F", at, "-framework", "daegun"]
            run(clang + ["-fmodules", "-Wall", "-Wquoted-include-in-framework-header", "-Werror",
                         os.path.join(work, "use.m"), "-o", f"{out}-use"])
            if os_part not in ("macos", "ios-macabi"):
                linked.append(triple)
                continue
            run([f"{out}-use"], what=f"{triple} module import")
            run(clang + ["-std=c11", "-I", os.path.join(at, "daegun.framework", "Headers"), ROUNDTRIP, "-o",
                         f"{out}-rt"])
            run([f"{out}-rt", FONT], what=f"{triple} round trip through the framework")
            ran.append(triple)
    swift = swift_package(archive, work)
    return (f"@import daegun built for {len(ran) + len(linked)} slices, run with the round trip on "
            f"{', '.join(ran)}; {swift}")


def swift_package(archive, work):
    # A SwiftPM package taking the XCFramework's zip as a binary target, as an app's does once it has downloaded it,
    # built and run on macOS.
    package = os.path.join(work, "swift")
    os.makedirs(os.path.join(package, "Sources", "use"))
    shutil.copyfile(archive, os.path.join(package, os.path.basename(archive)))
    with open(os.path.join(package, "Package.swift"), "w", encoding="utf-8") as f:
        f.write(SWIFT_PACKAGE.replace("ZIP", os.path.basename(archive)))
    with open(os.path.join(package, "Sources", "use", "main.swift"), "w", encoding="utf-8") as f:
        f.write(SWIFT_MAIN)
    run(["swift", "build", "-c", "release", "--package-path", package], what="swift build")
    run([os.path.join(package, ".build", "release", "use")], what="the Swift package's program")
    return "a SwiftPM package taking its zip as a binary target built and ran"


def image(kind):
    name, dockerfile = IMAGES[kind]
    if subprocess.run(["docker", "image", "inspect", name], capture_output=True).returncode != 0:
        made = subprocess.run(["docker", "build", "-t", name, "-"], input=dockerfile, capture_output=True, text=True)
        if made.returncode != 0:
            raise Failed(f"the {name} image doesn't build:\n{made.stderr[-2000:]}")
    return name


def linux_symbols(library):
    # A crash report and a profiler read the symbol table, which zig would strip with the debug info.
    folder = os.path.dirname(shutil.which(release.llvm_rc()))
    symbols = subprocess.run([os.path.join(folder, "llvm-nm"), "--defined-only", library], capture_output=True,
                             text=True, check=True).stdout.count("\n")
    sections = subprocess.run([os.path.join(folder, "llvm-readobj"), "--sections", library], capture_output=True,
                              text=True, check=True).stdout
    if symbols < 1000 or ".debug_" in sections:
        raise Failed(f"{library} has {symbols} symbols and {'debug info' if '.debug_' in sections else 'none'}")


def linux(kind, prefix, work):
    # In the image, as a user of it builds: pkg-config, shared where one ships, static, -static on musl, and CMake.
    inside = lambda path: "/src/" + os.path.relpath(path, ROOT)
    script = [
        "set -e", f"P={inside(prefix)}", f"W={inside(work)}", "export PKG_CONFIG_LIBDIR=$P/lib/pkgconfig",
        "CC='gcc -std=c11 -Wall -Wextra -Werror'",
        'if [ -e "$P/lib/libdaegun.so" ]; then',
        f'  $CC {ROUNDTRIP} $(pkg-config --cflags --libs daegun) -Wl,-rpath,$P/lib -o $W/rt-shared',
        f"  $W/rt-shared {FONT} >/dev/null", "fi",
        "S=$W/static && cp -a $P $S && rm -f $S/lib/libdaegun.so*",
        f"$CC {ROUNDTRIP} $(PKG_CONFIG_LIBDIR=$S/lib/pkgconfig pkg-config --static --cflags --libs daegun) "
        "-o $W/rt-static",
        f"$W/rt-static {FONT} >/dev/null"]
    if kind == "musl":
        script += [f"$CC -static {ROUNDTRIP} $(pkg-config --static --cflags --libs daegun) -o $W/rt-alone",
                   f"$W/rt-alone {FONT} >/dev/null"]
    script += [f"cmake -S $W/cmake -B $W/cmake-build -DCMAKE_PREFIX_PATH=$P -DROUNDTRIP=/src/{ROUNDTRIP} >/dev/null",
               "cmake --build $W/cmake-build >/dev/null",
               "for t in $W/cmake-build/rt-*; do $t " + FONT + " >/dev/null; done"]
    cmake_project(work)
    with open(os.path.join(work, "use.sh"), "w", encoding="utf-8") as f:
        f.write("\n".join(script) + "\n")
    run(["docker", "run", "--rm", "-v", f"{ROOT}:/src", "-w", "/src", "-e", "DAEGUN_ROOT=/src", image(kind),
         "sh", inside(os.path.join(work, "use.sh"))], what=f"the {kind} round trip in docker")
    static = "static and -static" if kind == "musl" else "static"
    shared = "shared and " if os.path.exists(os.path.join(prefix, "lib", "libdaegun.so")) else ""
    return f"pkg-config {shared}{static}, and CMake, ran in {IMAGES[kind][0]}"


def zig_link(triple, prefix, work):
    # An architecture docker can't run here: linked as a user of it would, through pkg-config, with zig for gcc. On
    # musl gcc's driver brings libgcc's unwinder, which zig's links for C only when asked, so its own stands in.
    alone = static_copy(prefix, os.path.join(work, "static"))
    driver = ["-lunwind"] if triple.endswith("musl") else []
    run(["zig", "cc", "-target", triple, "-std=c11", "-Wall", "-Wextra", "-Werror", ROUNDTRIP,
         *pkg(alone, "--static", "--cflags", "--libs"), *driver, "-o", os.path.join(work, "rt")])
    return f"the round trip links for {triple} through pkg-config with zig, which this machine can't run"


def android(target, prefix, work):
    # Linked with the NDK's clang at Rust's oldest Android, through pkg-config, CMake and the README's line. Nothing
    # here runs Android, so the outputs are read: 16 KB pages where Google Play asks, and the libdaegun.so each needs.
    clang, readelf = release.ndk_clang(target), release.ndk_tool("llvm-readelf")
    nm = release.ndk_tool("llvm-nm")
    alone = static_copy(prefix, os.path.join(work, "static"))
    static, shared = os.path.join(work, "rt-static"), os.path.join(work, "rt-shared")
    cc = [clang, "-std=c11", "-Wall", "-Wextra", "-Werror", ROUNDTRIP]
    run(cc + [*pkg(alone, "--static", "--cflags", "--libs"), "-o", static], what="the NDK's clang, static")
    run(cc + [*pkg(prefix, "--cflags", "--libs"), "-o", shared], what="the NDK's clang, shared")
    archive = [*pkg(alone, "--cflags"), os.path.join(alone, "lib", "libdaegun.a"),
               *readme(r"^\| Android \| `[^`]*libdaegun\.a ?([^`]*)` \|$")]
    run(cc + archive + ["-o", os.path.join(work, "rt-row")], what="the README's Android line")
    hide = readme(r"`(-Wl,--exclude-libs,[^`]*)`")
    exported = []
    for flags in ([], hide):
        plugin = os.path.join(work, "libplugin.so")
        run(cc + ["-shared", "-fPIC", *archive, *flags, "-o", plugin], what="an app's shared library")
        exported.append(sum(1 for line in run([nm, "-D", "--defined-only", plugin]).splitlines()
                            if re.search(r" (daegun_|rust_eh_personality$)", line)))
    if exported[0] == 0 or exported[1] != 0:
        raise Failed(f"an app's shared library exports {exported[1]} of daegun's symbols with "
                     f"{' '.join(hide)}, {exported[0]} without it")
    # The NDK's toolchain looks for packages in its sysroot alone, so an app names the folder with daegun_DIR.
    build = os.path.join(work, "cmake-build")
    run(["cmake", "-S", cmake_project(work), "-B", build,
         f"-DCMAKE_TOOLCHAIN_FILE={release.ndk()[0]}/build/cmake/android.toolchain.cmake",
         f"-DANDROID_ABI={ANDROID_ABI[target.split('-')[0]]}",
         f"-DANDROID_PLATFORM={release.ANDROID_API}", f"-Ddaegun_DIR={prefix}/lib/cmake/daegun",
         f"-DROUNDTRIP={os.path.join(ROOT, ROUNDTRIP)}"], what="CMake with the NDK's toolchain")
    run(["cmake", "--build", build], what="CMake's build for Android")
    needs = {static: False, shared: True, os.path.join(build, "rt-static"): False,
             os.path.join(build, "rt-daegun"): True}
    # Of 64-bit code alone: 32-bit Arm devices keep 4 KB pages.
    wide = release.pointer_size(target) == 8
    for elf in (os.path.join(prefix, "lib", "libdaegun.so"), *needs) if wide else ():
        aligns = [int(line.split()[-1], 16) for line in run([readelf, "-lW", elf]).splitlines()
                  if line.split()[:1] == ["LOAD"]]
        if not aligns or min(aligns) < 0x4000:
            raise Failed(f"{os.path.basename(elf)}'s segments align to {min(aligns, default=0):#x}, under 16 KB")
    for elf, needed in needs.items():
        if ("Shared library: [libdaegun.so]" in run([readelf, "-dW", elf])) != needed:
            raise Failed(f"{os.path.relpath(elf, work)} {'lacks' if needed else 'needs'} libdaegun.so by that name")
    pages = ", every segment on 16 KB pages" if wide else ""
    return (f"the round trip links with the NDK for {target}, static and shared through pkg-config and CMake and as "
            f"the README says{pages}, and {' '.join(hide)} hides {exported[0]} symbols")


def version_string(dll, key):
    # One value of a version resource's StringFileInfo: its key in UTF-16 and a NUL, padded to 32 bits, then the value
    # up to its NUL.
    with open(dll, "rb") as f:
        data = f.read()
    at = data.find((key + "\0").encode("utf-16-le"))
    if at < 0:
        return None
    at = (at + 2 * len(key) + 2 + 3) & ~3
    end = at
    while data[end:end + 2] != b"\0\0":
        end += 2
    return data[at:end].decode("utf-16-le")


def windows(arch, prefix, work):
    # The DLL's version resource as Explorer shows it: the archive's own release, and the holder and the copyright
    # LICENSE names.
    version = os.path.basename(prefix).split("-")[1]
    with open(os.path.join(ROOT, "LICENSE"), encoding="utf-8") as f:
        line = re.search(r"^Copyright \(c\) \S+ (.+)$", f.read(), re.M)
    want = {"CompanyName": line.group(1), "LegalCopyright": line.group(0), "FileVersion": version,
            "ProductVersion": version}
    name = f"daegun-{release.abi_version()[0]}.dll"
    got = {key: version_string(os.path.join(prefix, "bin", name), key) for key in want}
    if got != want:
        raise Failed(f"{name}'s version resource reads {got}, where it should read {want}")
    # cl under Wine, from the engine root with relative paths, as msvc-wine reads a /U… path as an option. The
    # static library takes what its .pc names; the DLL its import library and DAEGUN_DLL.
    cl = os.path.expanduser(f"~/opt/msvc/bin/{arch}/cl")
    rel = lambda path: os.path.relpath(path, ROOT)
    flags = pkg(prefix, "--cflags", "--libs")
    includes = [a for f in flags if f.startswith("-I") for a in ("/I", rel(f[2:]))]
    libs = [f"{f[2:]}.lib" for f in flags if f.startswith("-l")]
    libpath = [f"/LIBPATH:{rel(f[2:])}" for f in flags if f.startswith("-L")]
    common = [cl, "/nologo", "/std:c11", "/W4", "/WX", "/MD", *includes, ROUNDTRIP]
    env = {"WINEDEBUG": "-all", "DAEGUN_ROOT": "."}
    static, dll = os.path.join(work, "rt-static.exe"), os.path.join(work, "rt-dll.exe")
    run(common + [f"/Fe{rel(static)}", f"/Fo{rel(work)}/", *libs, "/link", *libpath], env=env, what="cl, static")
    shutil.copyfile(os.path.join(prefix, "bin", name), os.path.join(work, name))
    run(common + ["/DDAEGUN_DLL", f"/Fe{rel(dll)}", f"/Fo{rel(work)}/", rel(os.path.join(prefix, "lib",
                                                                                         f"{name}.lib"))],
        env=env, what="cl, DLL")
    # A program of any architecture loads the DLL by the name its import library gives, which no run checks on arm64.
    readobj = os.path.join(os.path.dirname(shutil.which(release.llvm_rc())), "llvm-readobj")
    imports = subprocess.run([readobj, "--coff-imports", dll], capture_output=True, text=True, check=True).stdout
    if f"Name: {name}" not in imports:
        raise Failed(f"a program linking {name}'s import library doesn't load it: {imports[:200]}")
    # A /MT program carries its own VC runtime, so with Wine's turned off it loads only if the DLL needs none.
    alone = os.path.join(work, "rt-dll-mt.exe")
    run([cl, "/nologo", "/std:c11", "/W4", "/WX", "/MT", *includes, ROUNDTRIP, "/DDAEGUN_DLL", f"/Fe{rel(alone)}",
         f"/Fo{rel(work)}/", rel(os.path.join(prefix, "lib", f"{name}.lib"))], env=env, what="cl /MT, DLL")
    # CMake drives cl through Ninja as a Windows project would, the C standard coming from daegun's targets alone.
    build = os.path.join(work, "cmake-build")
    cmake_env = {**env, "CC": "cl", "PATH": f"{os.path.dirname(cl)}:{os.environ['PATH']}"}
    run(["cmake", "-S", cmake_project(work), "-B", build, "-G", "Ninja", "-DCMAKE_SYSTEM_NAME=Windows",
         "-DCMAKE_BUILD_TYPE=Release", f"-DCMAKE_PREFIX_PATH={prefix}", f"-DROUNDTRIP={os.path.join(ROOT, ROUNDTRIP)}"],
        env=cmake_env, what=f"CMake for Windows {arch}")
    run(["cmake", "--build", build], env=cmake_env, what=f"CMake's build for Windows {arch}")
    shutil.copyfile(os.path.join(prefix, "bin", name), os.path.join(build, name))
    if arch != "x64":
        return (f"cl {arch} builds the round trip against the static library and {name}, /MD and /MT, and "
                "through CMake's two targets")
    for exe in (static, dll, *(os.path.join(build, f"rt-{kind}.exe") for kind in ("static", "daegun"))):
        run(["wine", rel(exe), FONT], env=env, what=f"{rel(exe)} under Wine")
    run(["wine", rel(alone), FONT], env={**env, "WINEDLLOVERRIDES": "vcruntime140,vcruntime140_1=d"},
        what=f"the /MT round trip against {name}, with no VC runtime")
    return (f"cl builds the round trip against the static library and {name}, directly and through CMake's two "
            "targets, and each runs under Wine, the DLL too in a /MT program with no VC runtime there; the DLL's "
            f"version resource reads {version}, {line.group(1)} and the copyright")


def extract(archive, work):
    os.makedirs(work)
    if archive.endswith(".zip"):
        run(["unzip", "-q", archive, "-d", work])
    else:
        run(["tar", "xzf", archive, "-C", work])
    entries = os.listdir(work)
    return os.path.join(work, entries[0]) if len(entries) == 1 else work


def use(archive, work):
    name = os.path.basename(archive)
    target = name.split("-", 2)[2].rsplit(".tar.gz", 1)[0].rsplit(".zip", 1)[0]
    prefix = extract(archive, os.path.join(work, "files"))
    # An archive unpacks into a prefix, as cargo-c's install does, beside anything else there.
    stray = sorted(set(os.listdir(prefix)) - {"bin", "include", "lib", "share"})
    if target != "xcframework" and stray:
        raise Failed(f"{name} has {stray} at its root, which would clash in a prefix")
    if release.TARGETS.get(target) == "linux":
        linux_symbols(os.path.join(prefix, "lib", "libdaegun.so"))
    docker = subprocess.run(["docker", "info", "--format", "{{.Architecture}}"], capture_output=True, text=True)
    docker_arch = {"arm64": "aarch64", "amd64": "x86_64"}.get(docker.stdout.strip(), docker.stdout.strip())
    if target == "xcframework":
        return xcframework(os.path.join(prefix, "daegun.xcframework"), archive, work)
    if target == "universal-apple-darwin":
        return f"{macos(prefix, work, ['arm64', 'x86_64'])}; {versions(prefix, work)}"
    if target.endswith("-apple-darwin"):
        return macos(prefix, work, ["arm64" if target.startswith("aarch64") else "x86_64"])
    if target in APPLE:
        return apple_static(target, prefix, work)
    if target.endswith("-windows-msvc"):
        return windows("arm64" if target.startswith("aarch64") else "x64", prefix, work)
    if release.TARGETS.get(target) == "android":
        return android(target, prefix, work)
    kind = "musl" if target.endswith("musl") else "gnu"
    if target.startswith(docker_arch + "-") and docker.returncode == 0:
        return linux(kind, prefix, work)
    return zig_link(f"{target.split('-')[0]}-linux-{'musl' if kind == 'musl' else 'gnu.2.17'}", prefix, work)


def main(argv):
    if len(argv) > 1 or argv[:1] in (["-h"], ["--help"]):
        print(USAGE)
        return 0 if argv[:1] in (["-h"], ["--help"]) else 1
    folder = os.path.abspath(argv[0]) if argv else os.path.join(ROOT, "target", "release-check")
    archives = sorted(n for n in os.listdir(folder) if n.startswith("daegun-") and n.endswith((".tar.gz", ".zip")))
    shutil.rmtree(WORK, ignore_errors=True)
    failed = 0
    for name in archives:
        try:
            stem = name.removesuffix(".tar.gz").removesuffix(".zip")
            said = use(os.path.join(folder, name), os.path.join(WORK, stem))
            print(f"  {name}: {said}", flush=True)
        except Failed as e:
            failed += 1
            print(f"  {name}: {e}", flush=True)
    print(f"{len(archives) - failed} of {len(archives)} archives used as a user would, each as far as this machine can")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
