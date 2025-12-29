#!/bin/bash
# macOS Development Build with Process Tap Entitlements
# Similar to `cargo tauri dev` but with proper code signing for Process Tap API
#
# Usage: ./scripts/dev-macos.sh [options]
#
# Options:
#   (no flags)           Full build + sign + reset permissions
#   --skip-build         Just run existing app (preserves permissions)
#   --run-only           Same as --skip-build (preserves permissions)
#   --reset-perms        Force reset permissions (use with --skip-build if needed)
#
# Permissions required for Process Tap API:
#   - Microphone (audio input access)
#   - Screen Recording (includes "System Audio Recording")
#
# Prerequisites:
#   - Node.js 18+ (use `nvm use 18` if needed)
#   - Admin password (for tccutil reset, only on full build or --reset-perms)
#
# Logs are automatically written to ~/gecko-debug.log

set -e

echo "🦎 Gecko macOS Development"
echo "=========================="

APP_PATH="target/debug/bundle/macos/Gecko.app"
ENTITLEMENTS="src-tauri/Gecko.entitlements"
LOG_FILE="$HOME/gecko-debug.log"

# Parse arguments
SKIP_BUILD=false
RUN_ONLY=false
RESET_PERMS=false
for arg in "$@"; do
    case $arg in
        --skip-build) SKIP_BUILD=true ;;
        --run-only) RUN_ONLY=true ;;
        --reset-perms) RESET_PERMS=true ;;
    esac
done

# Check Node version
NODE_VERSION=$(node -v 2>/dev/null | cut -d'.' -f1 | tr -d 'v')
if [ "$NODE_VERSION" -lt 18 ] 2>/dev/null; then
    echo "❌ Node.js 18+ required (current: $(node -v))"
    echo "   Run: nvm use 18"
    exit 1
fi

# Kill any running Gecko instance
echo ""
echo "🔪 Stopping any running Gecko..."
pkill -f gecko_ui 2>/dev/null || true
sleep 1

# Determine what to do based on flags
if [ "$RUN_ONLY" = true ] || [ "$SKIP_BUILD" = true ]; then
    echo ""
    if [ "$RUN_ONLY" = true ]; then
        echo "⏭️  Skipping build and sign (--run-only)"
    else
        echo "⏭️  Skipping build and sign (--skip-build)"
    fi
    echo "   (Permissions preserved from previous run)"
else
    # Full build: build + sign + reset permissions

    # Step 1: Build frontend
    echo ""
    echo "📦 Building frontend..."
    pnpm build

    # Step 2: Build Rust debug bundle
    echo ""
    echo "🔨 Building debug bundle..."
    cargo tauri build --debug --bundles app 2>&1 | grep -E "(Compiling|Finished|Bundling|Built|Error)" | tail -10

    # Step 3: Sign with entitlements
    # Ad-hoc signing creates NEW identity each time - permissions won't persist
    echo ""
    echo "🔐 Signing with entitlements..."
    codesign --force --deep --sign - --entitlements "$ENTITLEMENTS" "$APP_PATH"
    echo "   ✓ Signed with Process Tap entitlements"

    # Full build + sign creates new identity - must reset permissions
    RESET_PERMS=true
fi

# Step 4: Reset permissions (only on full build or explicit --reset-perms)
# Ad-hoc signing creates new identity each build, so permissions don't persist
# Process Tap API requires Microphone AND ScreenCapture (which includes System Audio Recording)
if [ "$RESET_PERMS" = true ]; then
    echo ""
    echo "🔓 Resetting ALL relevant permissions..."
    echo "   (You may be prompted for your password)"

    # Reset all audio-related permissions
    sudo tccutil reset Microphone
    sudo tccutil reset ScreenCapture
    sudo tccutil reset ListenEvent 2>/dev/null || true

    # Also reset ALL permissions for our specific app bundle (catches edge cases)
    sudo tccutil reset All com.gecko.gecko 2>/dev/null || true

    echo "   ✓ Microphone permissions reset"
    echo "   ✓ ScreenCapture permissions reset (includes System Audio Recording)"
    echo "   ✓ ListenEvent permissions reset"
    echo "   ✓ All permissions reset for com.gecko.gecko"
    echo ""
    echo "   ⚠️  You will need to grant permissions when the app starts:"
    echo "      1. Microphone - dialog will appear"
    echo "      2. Screen & System Audio Recording - System Settings opens, toggle Gecko ON"
    echo "      3. RESTART the app after granting Screen Recording permission"
else
    echo ""
    echo "⏭️  Skipping permission reset (use --reset-perms to force)"
fi

echo ""
echo "================================"
echo "🚀 Launching Gecko..."
echo ""
echo "   App logs: $LOG_FILE"
echo "   View logs: tail -f $LOG_FILE"
echo "================================"
echo ""

# Clear old log file
> "$LOG_FILE"

# Launch app properly via 'open' so Gecko gets permission prompts (not Terminal)
open "$APP_PATH"

echo "Gecko launched. Watching logs..."
echo "(Press Ctrl+C to stop watching)"
echo ""

# Follow the log file
tail -f "$LOG_FILE"
