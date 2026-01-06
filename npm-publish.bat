@echo off
setlocal

set VERSION=%1
if "%VERSION%"=="" (
    echo Usage: npm-publish.bat ^<version^>
    echo Example: npm-publish.bat 1.0.0
    exit /b 1
)

echo Building release...
cargo build --release

set PKG_DIR=target\release\package

echo Preparing npm package...
rmdir /s /q %PKG_DIR% 2>nul
mkdir %PKG_DIR%
copy target\release\llbot.exe %PKG_DIR%\
echo {"name": "llbot-cli-win-x64", "version": "%VERSION%", "description": "LLBot CLI launcher for Windows x64"} > %PKG_DIR%\package.json

cd target\release
tar -czf llbot-cli-win-x64.tgz package
npm login
npm publish llbot-cli-win-x64.tgz --access public

echo Done!
