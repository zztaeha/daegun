@echo off
rem MSVC's AddressSanitizer, which can't run under Wine, over the C round trip, the README's C and the C++ handles:
rem C instrumented against the plain library, then both sides against nightly's, as the gate's macOS steps are.
setlocal EnableExtensions
set "WIN=x86_64-pc-windows-msvc"
set "FONT=assets\test-fonts\inter\InterVariable.ttf"
set "OUT=target\msvc-asan"
set "RUSTFLAGS="

where cl >nul 2>&1 || call :vcvars || exit /b 1
if defined VSCMD_ARG_TGT_ARCH if /i not "%VSCMD_ARG_TGT_ARCH%"=="x64" goto :not64
set "THUNK=%VCToolsInstallDir%lib\x64\clang_rt.asan_dynamic_runtime_thunk-x86_64.lib"
if defined VCToolsInstallDir if not exist "%THUNK%" goto :noasan
rem After vcvars, which may move to %USERPROFILE%\source.
cd /d "%~dp0..\..\.."
where cargo >nul 2>&1 || goto :nocargo
rustup target add %WIN% >nul 2>&1
if not exist %OUT% mkdir %OUT%

echo.
echo [1/2] C instrumented, the library plain
set "TOOLCHAIN="
call :lib %OUT%\plain || goto :fail
call :build %OUT%\plain || goto :fail
call :run %OUT%\plain || goto :fail

echo.
echo [2/2] both sides instrumented, the library on nightly and keeping no spare handles
rustup run nightly rustc --version >nul 2>&1 || rustup toolchain install nightly --profile minimal || goto :fail
rustup target add %WIN% --toolchain nightly >nul 2>&1
set "TOOLCHAIN=+nightly"
set "RUSTFLAGS=-Zsanitizer=address --cfg daegun_no_spares"
call :lib %OUT%\rust || goto :fail
call :build %OUT%\rust || goto :fail
call :run %OUT%\rust || goto :fail
call :freed %OUT%\rust || goto :fail

echo.
echo MSVC ASan clean on both, and a path used after its free and one freed twice were reported
exit /b 0

rem Each toolchain names the system libraries its standard library needs; nightly's may differ from stable's.
:lib
echo building the library, a few minutes the first time
cargo %TOOLCHAIN% rustc --features capi --crate-type staticlib --target %WIN% --target-dir %~1 ^
  -- --print native-static-libs 2>"%~1.log" || (type "%~1.log" & exit /b 1)
set "LIBS=%~1\%WIN%\debug\daegun.lib"
for /f "tokens=2* delims=:" %%a in ('findstr /c:"native-static-libs:" "%~1.log"') do set "LIBS=%LIBS% %%b"
exit /b 0

:build
set "FLAGS=/nologo /fsanitize=address /Zi /MD /W4 /I src\api\c\include /Fo%~1\ /Fd%~1\"
cl /std:c11 %FLAGS% src\api\c\tests\roundtrip.c /Fe%~1\rt.exe /link %LIBS% || exit /b 1
cl /std:c11 %FLAGS% src\api\c\tests\readme.c /Fe%~1\readme.exe /link %LIBS% || exit /b 1
cl /std:c++17 /EHsc %FLAGS% src\api\c\tests\handles.cpp /Fe%~1\handles.exe /link %LIBS% || exit /b 1
exit /b 0

:run
"%~1\rt.exe" %FONT% || exit /b 1
"%~1\readme.exe" %FONT% || exit /b 1
"%~1\handles.exe" || exit /b 1
exit /b 0

rem The library keeps no spare handles here, so a path used after its free, or freed twice, must be reported.
:freed
set "C=%~1\freed.c"
>"%C%" echo #include "daegun.h"
>>"%C%" echo int main(int argc, char **argv) {
>>"%C%" echo     (void)argv;
>>"%C%" echo     daegun_path *p = daegun_path_new();
>>"%C%" echo     daegun_path_free(p);
>>"%C%" echo     if (argc ^> 1) daegun_path_free(p); else daegun_path_move_to(p, 1.0f, 2.0f);
>>"%C%" echo     return 0;
>>"%C%" echo }
cl /std:c11 %FLAGS% "%C%" /Fe%~1\freed.exe /link %LIBS% || exit /b 1
"%~1\freed.exe" >"%~1\freed.log" 2>&1 && goto :unseen
findstr /c:"heap-use-after-free" "%~1\freed.log" >nul || goto :unseen
"%~1\freed.exe" twice >"%~1\freed.log" 2>&1 && goto :unseen
findstr /c:"double-free" /c:"heap-use-after-free" "%~1\freed.log" >nul || goto :unseen
exit /b 0

:unseen
type "%~1\freed.log"
echo a path used after its free, or freed twice, went unseen; try again after
echo   set ASAN_OPTIONS=windows_hook_rtl_allocators=true
exit /b 1

:vcvars
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VCVARS="
if exist "%VSWHERE%" "%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 ^
  -find VC\Auxiliary\Build\vcvars64.bat >"%TEMP%\daegun-vcvars.txt"
if exist "%TEMP%\daegun-vcvars.txt" set /p VCVARS=<"%TEMP%\daegun-vcvars.txt"
del "%TEMP%\daegun-vcvars.txt" >nul 2>&1
if defined VCVARS call "%VCVARS%" >nul
where cl >nul 2>&1 && exit /b 0
echo no cl: install Visual Studio's "Desktop development with C++", or run this from its x64 Native Tools prompt
exit /b 1

:noasan
set "SETUP=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\setup.exe"
echo MSVC's AddressSanitizer isn't installed: in the Visual Studio Installer, choose Modify, then
echo Individual components, tick "C++ AddressSanitizer" and run this again. Or, as administrator:
echo   "%SETUP%" modify --installPath "%VSINSTALLDIR:~0,-1%" --add Microsoft.VisualStudio.Component.VC.ASAN --passive
exit /b 1

:not64
echo this prompt builds for %VSCMD_ARG_TGT_ARCH%: run it from an x64 Native Tools prompt, or a plain one
exit /b 1

:nocargo
echo no cargo: install Rust from https://rustup.rs
exit /b 1

:fail
echo.
echo MSVC ASan failed, see above
exit /b 1
