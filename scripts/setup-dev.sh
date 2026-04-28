#!/usr/bin/env bash
set -Eeuo pipefail

COMPOSE_FILE="docker-compose.dev.yml"
CONFIG_FILE="config/docker_compose.dev.toml"
DASHBOARD_CONFIG_FILE="config/dashboard.dev.toml"

API_URL="https://api-dev.dfl24.com"
DASHBOARD_URL="https://dashboard-dev.dfl24.com"
SDK_URL="https://pay-dev.dfl24.com"
MONITORING_URL="https://monitoring-dev.dfl24.com"

# Set up error logging - redirect stderr to both log file and console
ERROR_LOG="error.log"
exec 2> >(tee -a "${ERROR_LOG}" >&2)

# Set traps for errors and interruptions
trap 'handle_error "$LINENO" "$BASH_COMMAND" "$?"' ERR
trap 'handle_interrupt' INT TERM

# Variables for installation status
VERSION="unknown"
INSTALLATION_STATUS="initiated"
SCARF_PARAMS=()

handle_error() {
    local lineno=$1
    local last_command=$2
    local exit_code=$3

    local log_content=""
    if [ -f "${ERROR_LOG}" ] && [ -s "${ERROR_LOG}" ]; then
        log_content=$(tail -n 1 "${ERROR_LOG}" | tr '\n' '|' | sed 's/|$//')
    fi

    INSTALLATION_STATUS="error"
    ERROR_MESSAGE="Command '\$ ${last_command}' failed at line ${lineno} with exit code ${exit_code} and error logs: ${log_content:-'not available'}"

    SCARF_PARAMS+=(
        "error_type=script_error"
        "error_message=${ERROR_MESSAGE}"
        "error_code=${exit_code}"
    )

    scarf_call
    cleanup
    exit $exit_code
}

handle_interrupt() {
    echo ""
    echo_warning "Script interrupted by user"
    INSTALLATION_STATUS="user_interrupt"
    scarf_call
    cleanup
    exit 130
}

cleanup() {
    if [ -n "${PROFILE:-}" ]; then
        echo_info "Cleaning up any started containers..."
        case $PROFILE in
        standalone)
            $DOCKER_COMPOSE -f "${COMPOSE_FILE}" down >/dev/null 2>&1 || true
            ;;
        standard)
            $DOCKER_COMPOSE -f "${COMPOSE_FILE}" down >/dev/null 2>&1 || true
            ;;
        full)
            $DOCKER_COMPOSE -f "${COMPOSE_FILE}" --profile scheduler --profile monitoring --profile olap --profile full_setup down >/dev/null 2>&1 || true
            ;;
        esac
    fi

    if [ -f "${ERROR_LOG}" ]; then
        rm -f "${ERROR_LOG}"
    fi
}

# ANSI color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

echo_info()    { printf "${BLUE}[INFO]${NC} %s\n" "$1"; }
echo_success() { printf "${GREEN}[SUCCESS]${NC} %s\n" "$1"; }
echo_warning() { printf "${YELLOW}[WARNING]${NC} %s\n" "$1"; }
echo_error()   { printf "${RED}[ERROR]${NC} %s\n" "$1"; }

show_banner() {
    printf "${BLUE}${BOLD}\n"
    printf "  Hyperswitch — DEV environment (dfl24.com)\n"
    printf "${NC}\n"
}

detect_docker_compose() {
    if command -v docker &>/dev/null; then
        CONTAINER_ENGINE="docker"
        echo_success "Docker is installed."
    elif command -v podman &>/dev/null; then
        CONTAINER_ENGINE="podman"
        echo_success "Podman is installed."
    else
        echo_error "Neither Docker nor Podman is installed."
        exit 1
    fi

    if $CONTAINER_ENGINE compose version &>/dev/null; then
        DOCKER_COMPOSE="${CONTAINER_ENGINE} compose"
        echo_success "Compose is installed for ${CONTAINER_ENGINE}."
    else
        echo_error "Compose is not installed for ${CONTAINER_ENGINE}."
        exit 1
    fi
}

check_prerequisites() {
    if ! command -v curl &>/dev/null; then
        echo_error "curl is not installed."
        exit 1
    fi
    echo_success "curl is installed."

    required_ports=(8080 8081 9000 9050 5432 6379)
    unavailable_ports=()

    for port in "${required_ports[@]}"; do
        if command -v nc &>/dev/null; then
            if nc -z localhost "$port" 2>/dev/null; then
                unavailable_ports+=("$port")
            fi
        elif command -v lsof &>/dev/null; then
            if lsof -i :"$port" &>/dev/null; then
                unavailable_ports+=("$port")
            fi
        else
            echo_warning "Cannot check ports (nc/lsof not found). Skipping."
            break
        fi
    done

    if [ ${#unavailable_ports[@]} -ne 0 ]; then
        echo_warning "Ports already in use: ${unavailable_ports[*]}"
        echo -n "Continue anyway? (y/n): "
        read -n 1 -r REPLY
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi
}

setup_config() {
    if [ ! -f "${CONFIG_FILE}" ]; then
        echo_error "Config file '${CONFIG_FILE}' not found."
        exit 1
    fi
    if [ ! -f "${DASHBOARD_CONFIG_FILE}" ]; then
        echo_error "Dashboard config file '${DASHBOARD_CONFIG_FILE}' not found."
        exit 1
    fi

    local env_file=".oneclick-setup.env"
    echo "# Dev environment setup" >"${env_file}"
    echo "# Generated on $(date)" >>"${env_file}"
    echo "" >>"${env_file}"
    echo "ONE_CLICK_SETUP=true" >>"${env_file}"
}

select_profile() {
    printf "\nSelect a setup option:\n"
    printf "1) ${YELLOW}Standard Setup${NC} ${BLUE}[Recommended]${NC}: App Server, Control Center, PostgreSQL, Redis\n"
    printf "2) ${YELLOW}Full Stack Setup${NC}: Everything in Standard + Monitoring + Scheduler\n"
    printf "3) ${YELLOW}Standalone App Server${NC}: App Server, PostgreSQL, Redis only\n\n"

    local profile_selected=false
    while [ "${profile_selected}" = "false" ]; do
        echo -n "Enter your choice (1-3): "
        read -n 1 profile_choice
        echo
        case $profile_choice in
        1) PROFILE="standard"; profile_selected=true ;;
        2) PROFILE="full";     profile_selected=true ;;
        3) PROFILE="standalone"; profile_selected=true ;;
        *) echo_error "Invalid choice. Enter 1, 2, or 3." ;;
        esac
    done

    echo "Selected setup: ${PROFILE}"
}

scarf_call() {
    chmod +x scripts/notify_scarf.sh
    if [ ${#SCARF_PARAMS[@]} -eq 0 ]; then
        scripts/notify_scarf.sh "version=${VERSION}" "status=${INSTALLATION_STATUS}" >/dev/null 2>&1
    else
        scripts/notify_scarf.sh "version=${VERSION}" "status=${INSTALLATION_STATUS}" "${SCARF_PARAMS[@]}" >/dev/null 2>&1
    fi
    SCARF_PARAMS=()
}

start_services() {
    case $PROFILE in
    standalone)
        $DOCKER_COMPOSE -f "${COMPOSE_FILE}" --env-file .oneclick-setup.env up -d pg redis-standalone migration_runner hyperswitch-server
        ;;
    standard)
        $DOCKER_COMPOSE -f "${COMPOSE_FILE}" --env-file .oneclick-setup.env up -d
        ;;
    full)
        $DOCKER_COMPOSE -f "${COMPOSE_FILE}" --env-file .oneclick-setup.env --profile scheduler --profile monitoring --profile olap --profile full_setup up -d
        ;;
    esac
}

check_services_health() {
    local HYPERSWITCH_HEALTH_URL="http://localhost:8080/health"
    local HYPERSWITCH_DEEP_HEALTH_URL="http://localhost:8080/health/ready"
    local is_success=true

    health_response=$(curl --silent --fail "${HYPERSWITCH_HEALTH_URL}") || is_success=false
    if [ "${health_response}" != "health is good" ]; then
        is_success=false
    fi

    deep_health_response=$(curl --silent --fail "${HYPERSWITCH_DEEP_HEALTH_URL}") || is_success=false
    if [[ "$(echo "${deep_health_response}" | jq --raw-output '.error')" != "null" ]]; then
        is_success=false
    fi

    if [ "${is_success}" = true ]; then
        VERSION=$(curl --silent --output /dev/null --request GET --write-out '%header{x-hyperswitch-version}' "http://localhost:8080" | sed 's/-dirty$//')
        INSTALLATION_STATUS="success"
        scarf_call
    fi
    print_access_info
}

print_access_info() {
    printf "${BLUE}"
    printf "╔════════════════════════════════════════════════════════════════╗\n"
    printf "║        Hyperswitch — DEV environment ready!                    ║\n"
    printf "╚════════════════════════════════════════════════════════════════╝\n"
    printf "${NC}\n"

    printf "${GREEN}${BOLD}Services are accessible at:${NC}\n"

    if [ "$PROFILE" != "standalone" ]; then
        printf "  • ${GREEN}${BOLD}Control Center${NC}: ${BLUE}${BOLD}${DASHBOARD_URL}${NC}\n"
    fi
    printf "  • ${GREEN}${BOLD}App Server (API)${NC}: ${BLUE}${BOLD}${API_URL}${NC}\n"
    printf "  • ${GREEN}${BOLD}Web SDK${NC}: ${BLUE}${BOLD}${SDK_URL}/HyperLoader.js${NC}\n"
    if [ "$PROFILE" = "full" ]; then
        printf "  • ${GREEN}${BOLD}Monitoring (Grafana)${NC}: ${BLUE}${BOLD}${MONITORING_URL}${NC}\n"
    fi
    printf "\n"

    printf "            Default credentials:\n"
    printf "            Email:    demo@hyperswitch.com\n"
    printf "            Password: Hyperswitch@123\n"
    printf "\n"

    echo_info "To stop all services:"
    case $PROFILE in
    standalone|standard)
        printf "${BLUE}$DOCKER_COMPOSE -f ${COMPOSE_FILE} down${NC}\n"
        ;;
    full)
        printf "${BLUE}$DOCKER_COMPOSE -f ${COMPOSE_FILE} --profile scheduler --profile monitoring --profile olap --profile full_setup down${NC}\n"
        ;;
    esac
    printf "\n"
}

# Main
scarf_call
show_banner
detect_docker_compose
check_prerequisites
setup_config
source .oneclick-setup.env
select_profile
start_services
check_services_health
