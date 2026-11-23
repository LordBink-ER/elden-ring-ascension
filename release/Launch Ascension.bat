@echo off
rem ELDEN RING: ASCENSION - starts the game with this folder's me3 profile.
cd /d "%~dp0"
where me3 >nul 2>nul
if errorlevel 1 goto nome3
me3 launch --game eldenring --profile "%~dp0ascension.me3"
if errorlevel 1 pause
exit /b

:nome3
echo Mod Engine 3 - me3 - is not installed, or not on PATH.
echo Install it from https://github.com/garyttierney/me3/releases
echo then run this file again.
pause
exit /b 1
