#!/bin/bash
set -e

# ADIC Core Monitoring Script
# Real-time monitoring of ADIC network metrics

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Default values
REFRESH_RATE=2
NODE_URL="http://localhost:8080"
METRICS_URL="http://localhost:9090"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -r|--refresh)
            REFRESH_RATE="$2"
            shift 2
            ;;
        -n|--node)
            NODE_URL="$2"
            shift 2
            ;;
        -m|--metrics)
            METRICS_URL="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [options]"
            echo "Options:"
            echo "  -r, --refresh SECONDS    Refresh rate (default: 2)"
            echo "  -n, --node URL          Node API URL (default: http://localhost:8080)"
            echo "  -m, --metrics URL       Metrics URL (default: http://localhost:9090)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Clear screen function
clear_screen() {
    printf '\033[2J\033[H'
}

# Get node status
get_node_status() {
    curl -s "$NODE_URL/status" 2>/dev/null || echo "{}"
}

# Get metrics
get_metrics() {
    curl -s "$METRICS_URL/metrics" 2>/dev/null | grep "^adic_" || true
}

# Format number with commas
format_number() {
    printf "%'d" "$1" 2>/dev/null || echo "$1"
}

# Format duration
format_duration() {
    local seconds=$1
    local days=$((seconds / 86400))
    local hours=$(((seconds % 86400) / 3600))
    local mins=$(((seconds % 3600) / 60))
    local secs=$((seconds % 60))
    
    if [ $days -gt 0 ]; then
        printf "%dd %02dh %02dm %02ds" $days $hours $mins $secs
    elif [ $hours -gt 0 ]; then
        printf "%02dh %02dm %02ds" $hours $mins $secs
    elif [ $mins -gt 0 ]; then
        printf "%02dm %02ds" $mins $secs
    else
        printf "%ds" $secs
    fi
}

# Draw header
draw_header() {
    local width=$(tput cols)
    printf "${CYAN}%*s${NC}\n" $width | tr ' ' '='
    printf "${CYAN}%*s${NC}\n" $(((width + 28) / 2)) "ADIC NETWORK MONITOR"
    printf "${CYAN}%*s${NC}\n" $width | tr ' ' '='
}

# Draw section
draw_section() {
    local title=$1
    printf "\n${YELLOW}▶ %s${NC}\n" "$title"
    printf "%40s\n" | tr ' ' '-'
}

# Monitor loop
monitor() {
    while true; do
        clear_screen
        draw_header
        
        # Get current data
        STATUS=$(get_node_status)
        METRICS=$(get_metrics)
        
        # Parse status
        if [ -n "$STATUS" ] && [ "$STATUS" != "{}" ]; then
            NODE_NAME=$(echo "$STATUS" | jq -r '.node_name // "Unknown"')
            NODE_VERSION=$(echo "$STATUS" | jq -r '.version // "Unknown"')
            UPTIME=$(echo "$STATUS" | jq -r '.uptime // 0')
            PEERS=$(echo "$STATUS" | jq -r '.peers // 0')
            
            draw_section "NODE INFORMATION"
            printf "  ${GREEN}●${NC} Status:      ${GREEN}Online${NC}\n"
            printf "  Name:         %s\n" "$NODE_NAME"
            printf "  Version:      %s\n" "$NODE_VERSION"
            printf "  Uptime:       %s\n" "$(format_duration $UPTIME)"
            printf "  Peers:        %s\n" "$PEERS"
        else
            draw_section "NODE INFORMATION"
            printf "  ${RED}●${NC} Status:      ${RED}Offline${NC}\n"
        fi
        
        # Parse metrics
        if [ -n "$METRICS" ]; then
            # Extract key metrics
            MSG_TOTAL=$(echo "$METRICS" | grep "^adic_messages_total" | awk '{print $2}' | head -1)
            MSG_FINALIZED=$(echo "$METRICS" | grep "^adic_finalized_messages" | awk '{print $2}' | head -1)
            TIPS_COUNT=$(echo "$METRICS" | grep "^adic_tips_count" | awk '{print $2}' | head -1)
            KCORE_SIZE=$(echo "$METRICS" | grep "^adic_kcore_size" | awk '{print $2}' | head -1)
            
            draw_section "CONSENSUS METRICS"
            printf "  Total Messages:     %s\n" "$(format_number ${MSG_TOTAL:-0})"
            printf "  Finalized:          %s\n" "$(format_number ${MSG_FINALIZED:-0})"
            
            if [ -n "$MSG_TOTAL" ] && [ -n "$MSG_FINALIZED" ] && [ "$MSG_TOTAL" -gt 0 ]; then
                FINALIZATION_RATE=$(echo "scale=2; $MSG_FINALIZED * 100 / $MSG_TOTAL" | bc)
                printf "  Finalization Rate:  ${GREEN}%.2f%%${NC}\n" "$FINALIZATION_RATE"
            fi
            
            printf "  Current Tips:       %s\n" "$(format_number ${TIPS_COUNT:-0})"
            printf "  K-Core Size:        %s\n" "$(format_number ${KCORE_SIZE:-0})"
            
            # Throughput calculation
            MSG_RATE=$(echo "$METRICS" | grep "^adic_message_rate" | awk '{print $2}' | head -1)
            if [ -n "$MSG_RATE" ]; then
                draw_section "PERFORMANCE"
                printf "  Message Rate:       %.2f msg/s\n" "$MSG_RATE"
            fi
            
            # Error stats
            ERRORS=$(echo "$METRICS" | grep "^adic_errors_total" | awk '{print $2}' | head -1)
            if [ -n "$ERRORS" ] && [ "$ERRORS" -gt 0 ]; then
                printf "  ${RED}Errors:             %s${NC}\n" "$(format_number $ERRORS)"
            fi
        fi
        
        # Show last update time
        printf "\n${CYAN}Last updated: %s${NC}\n" "$(date '+%Y-%m-%d %H:%M:%S')"
        printf "Refresh rate: %ss (Press Ctrl+C to exit)\n" "$REFRESH_RATE"
        
        sleep "$REFRESH_RATE"
    done
}

# Trap Ctrl+C
trap 'printf "\n${GREEN}Monitoring stopped.${NC}\n"; exit 0' INT

# Check dependencies
if ! command -v curl &> /dev/null; then
    echo "Error: curl is required but not installed."
    exit 1
fi

if ! command -v jq &> /dev/null; then
    echo "Warning: jq is not installed. Some features may not work properly."
fi

# Start monitoring
monitor