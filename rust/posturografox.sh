#!/usr/bin/env bash
#
# posturografox.sh - Script de gestion para el port Rust de Posturografox
# Uso: ./posturografox.sh [comando]
# Sin argumentos: abre menu interactivo
#

set -uo pipefail

# ── Colores ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# ── Variables ────────────────────────────────────────────────────────────────
PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_NAME="posturografox"
VERSION=$(grep '^version' "$PROJECT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

_detect_cargo_target() {
    if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
        echo "$CARGO_TARGET_DIR"
    elif grep -q 'target-dir' ~/.cargo/config.toml 2>/dev/null; then
        grep 'target-dir' ~/.cargo/config.toml | head -1 | sed 's/.*= *"\(.*\)".*/\1/' | sed "s|~|$HOME|"
    else
        echo "$PROJECT_DIR/target"
    fi
}
CARGO_TARGET="$(_detect_cargo_target)"

find_binary() {
    local perfil="$1" # debug | release
    local bin="$CARGO_TARGET/$perfil/$BIN_NAME"
    [[ -f "$bin" ]] && echo "$bin" && return 0
    return 1
}

# ── Funciones auxiliares ─────────────────────────────────────────────────────
info()    { echo -e "${BLUE}[INFO]${NC} $*"; }
success() { echo -e "${GREEN}[OK]${NC} $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; }
header()  { echo -e "\n${BOLD}${CYAN}=== $* ===${NC}\n"; }

elapsed() {
    local start=$1
    local end=$(date +%s)
    local diff=$((end - start))
    echo "$((diff / 60))m $((diff % 60))s"
}

pause_after() {
    echo ""
    echo -e "${DIM}Presiona ENTER para volver al menu...${NC}"
    read -r
}

# ── Verificacion de dependencias ─────────────────────────────────────────────
check_deps() {
    header "Verificando dependencias"
    local missing=0

    for cmd in cargo rustc; do
        if command -v "$cmd" &>/dev/null; then
            success "$cmd -> $(command "$cmd" --version 2>/dev/null | head -1)"
        else
            error "$cmd no encontrado"
            missing=1
        fi
    done

    if [[ $missing -eq 1 ]]; then
        error "Falta instalar Rust (https://rustup.rs)"
        return 1
    fi
    success "Todas las dependencias disponibles"
}

# ── Desarrollo ───────────────────────────────────────────────────────────────
cmd_dev() {
    header "Modo desarrollo (cargo run)"
    check_deps || return
    cd "$PROJECT_DIR"
    info "Compilando y ejecutando en modo debug..."
    cargo run || true
}

# ── Build release ────────────────────────────────────────────────────────────
cmd_build() {
    header "Build Posturografox v$VERSION (release)"
    check_deps || return
    local start=$(date +%s)
    cd "$PROJECT_DIR"

    if ! cargo build --release; then
        error "Build falló después de $(elapsed "$start")"
        return 1
    fi
    success "Build completo en $(elapsed "$start")"
    info "Binario: $(find_binary release)"
}

cmd_build_debug() {
    header "Build debug"
    cd "$PROJECT_DIR"
    local start=$(date +%s)

    if ! cargo build; then
        error "Build debug falló después de $(elapsed "$start")"
        return 1
    fi
    success "Build debug en $(elapsed "$start")"
}

# ── Ejecutar binario ─────────────────────────────────────────────────────────
cmd_run() {
    header "Ejecutando Posturografox v$VERSION"
    local bin
    if ! bin="$(find_binary release)"; then
        warn "No hay binario release compilado, compilando primero..."
        cmd_build || return 1
        bin="$(find_binary release)" || { error "No se pudo generar el binario"; return 1; }
    fi
    "$bin" "$@" || true
}

# ── Limpiar ──────────────────────────────────────────────────────────────────
cmd_clean() {
    header "Limpieza"
    cd "$PROJECT_DIR"
    info "cargo clean (target en: $CARGO_TARGET)..."
    cargo clean
    success "Limpio"
}

# ── Info ─────────────────────────────────────────────────────────────────────
cmd_info() {
    header "Posturografox v$VERSION (Rust)"
    echo -e "${BOLD}Directorio:${NC}     $PROJECT_DIR"
    echo -e "${BOLD}Cargo target:${NC}   $CARGO_TARGET"
    echo -e "${BOLD}Rust:${NC}           $(rustc --version 2>/dev/null || echo 'N/A')"
    echo -e "${BOLD}Branch:${NC}         $(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null || echo 'N/A')"
    echo -e "${BOLD}Commit:${NC}         $(git -C "$PROJECT_DIR" log --oneline -1 2>/dev/null || echo 'N/A')"
    echo ""

    local bin_release bin_debug
    bin_release="$(find_binary release 2>/dev/null || echo '')"
    bin_debug="$(find_binary debug 2>/dev/null || echo '')"
    [[ -n "$bin_release" ]] && echo -e "${BOLD}Binario release:${NC} $bin_release" || warn "Sin binario release (usa 'build')"
    [[ -n "$bin_debug" ]] && echo -e "${BOLD}Binario debug:${NC}   $bin_debug"
}

# ══════════════════════════════════════════════════════════════════════════════
# ── MENU INTERACTIVO ─────────────────────────────────────────────────────────
# ══════════════════════════════════════════════════════════════════════════════

show_banner() {
    clear
    echo -e "${BOLD}${CYAN}"
    echo "  ____          _                          __      __     "
    echo " |  _ \ ___  __| |_ _   _ _ __ ___    ___  / _|_  _/ _|_  "
    echo " | |_) / _ \/ _\` | | | | | '__/ _ \  / _ \| |_ \ \/ / _ \ "
    echo " |  __/ (_) \__ \ | |_| | | | (_) || (_) |  _| >  <  __/ "
    echo " |_|   \___/|___/_|\__,_|_|  \___(_)\___/|_|  /_/\_\___| "
    echo -e "${NC}"
    echo -e "${DIM}  Posturografox (Rust) v$VERSION"
    echo -e "  $(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null || echo '-') · $(git -C "$PROJECT_DIR" log --oneline -1 2>/dev/null | cut -c1-50 || echo '-')${NC}"
    echo ""
}

show_menu() {
    echo -e "${BOLD} DESARROLLO${NC}"
    echo -e "  ${GREEN}1${NC})  Dev              ${DIM}cargo run (debug)${NC}"
    echo ""
    echo -e "${BOLD} BUILD${NC}"
    echo -e "  ${YELLOW}2${NC})  Build release    ${DIM}cargo build --release${NC}"
    echo -e "  ${YELLOW}3${NC})  Build debug      ${DIM}Sin optimizaciones${NC}"
    echo ""
    echo -e "${BOLD} GESTION${NC}"
    echo -e "  ${BLUE}4${NC})  Ejecutar app     ${DIM}Lanzar binario release (compila si falta)${NC}"
    echo -e "  ${BLUE}5${NC})  Info proyecto    ${DIM}Versiones y binarios${NC}"
    echo -e "  ${RED}6${NC})  Limpiar          ${DIM}cargo clean${NC}"
    echo ""
    echo -e "  ${BOLD}0${NC})  Salir"
    echo ""
}

menu_loop() {
    while true; do
        show_banner
        show_menu

        echo -ne "${BOLD}  Opcion: ${NC}"
        read -r choice

        case "${choice// /}" in
            1) cmd_dev;         pause_after ;;
            2) cmd_build;       pause_after ;;
            3) cmd_build_debug; pause_after ;;
            4) cmd_run;         pause_after ;;
            5) cmd_info;        pause_after ;;
            6) cmd_clean;       pause_after ;;
            0|q|salir) echo -e "\n${GREEN}Hasta luego${NC}"; exit 0 ;;
            "") ;;
            *) error "Opcion no valida: $choice"; sleep 1 ;;
        esac
    done
}

# ── Ayuda CLI ────────────────────────────────────────────────────────────────
cmd_help() {
    echo -e "${BOLD}${CYAN}Posturografox (Rust) v$VERSION${NC}"
    echo ""
    echo -e "${BOLD}Uso:${NC} ./posturografox.sh [comando]"
    echo -e "     ./posturografox.sh          ${DIM}(menu interactivo)${NC}"
    echo ""
    echo "  dev            cargo run (debug)"
    echo "  build          Build release"
    echo "  build:debug    Build debug"
    echo "  run            Ejecutar binario release (compila si falta)"
    echo "  info           Info del proyecto"
    echo "  clean          cargo clean"
    echo "  help           Esta ayuda"
}

# ── Router ───────────────────────────────────────────────────────────────────
main() {
    cd "$PROJECT_DIR"

    if [[ $# -eq 0 ]]; then
        menu_loop
        exit 0
    fi

    case "$1" in
        dev)         cmd_dev ;;
        build)       cmd_build ;;
        build:debug) cmd_build_debug ;;
        run)         shift; cmd_run "$@" ;;
        info)        cmd_info ;;
        clean)       cmd_clean ;;
        help|--help|-h) cmd_help ;;
        *) error "Comando desconocido: $1"; cmd_help; exit 1 ;;
    esac
}

main "$@"
