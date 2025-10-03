#!/usr/bin/env python3
"""
ADIC Event Stream Client - Python Example

This example demonstrates how to connect to ADIC's real-time event stream
using WebSocket or SSE with automatic fallback and reconnection.

Usage:
    python event_stream_client.py --node-url ws://localhost:9121
"""

import asyncio
import aiohttp
import json
import logging
import signal
import sys
from datetime import datetime
from typing import Dict, Any, Optional
from dataclasses import dataclass, field

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)


@dataclass
class EventStats:
    """Track event statistics"""
    total_events: int = 0
    events_by_type: Dict[str, int] = field(default_factory=dict)
    last_event_time: Optional[datetime] = None
    connection_start: Optional[datetime] = None

    def record_event(self, event_type: str):
        self.total_events += 1
        self.events_by_type[event_type] = self.events_by_type.get(event_type, 0) + 1
        self.last_event_time = datetime.now()

    def print_summary(self):
        if self.connection_start:
            duration = (datetime.now() - self.connection_start).total_seconds()
            events_per_min = (self.total_events / duration) * 60 if duration > 0 else 0

            logger.info("\n" + "="*60)
            logger.info("EVENT STATISTICS")
            logger.info("="*60)
            logger.info(f"Total Events:    {self.total_events}")
            logger.info(f"Events/min:      {events_per_min:.2f}")
            logger.info(f"Connection time: {duration:.1f}s")
            logger.info(f"\nEvents by type:")
            for event_type, count in sorted(self.events_by_type.items()):
                logger.info(f"  {event_type:25s} {count:6d}")
            logger.info("="*60 + "\n")


class EventStreamClient:
    """
    WebSocket client for ADIC event stream with automatic reconnection
    """

    def __init__(self, node_url: str, event_filter: str = "all", priority: str = "all"):
        self.node_url = node_url
        self.event_filter = event_filter
        self.priority = priority
        self.running = False
        self.stats = EventStats()
        self.reconnect_delay = 1.0
        self.max_reconnect_delay = 60.0

    async def connect_websocket(self) -> bool:
        """Connect to WebSocket endpoint"""
        try:
            # Build WebSocket URL
            ws_url = self.node_url.replace('http://', 'ws://').replace('https://', 'wss://')
            ws_url = f"{ws_url}/v1/ws/events"

            params = {
                'events': self.event_filter,
                'priority': self.priority
            }

            logger.info(f"Connecting to WebSocket: {ws_url}")
            logger.info(f"Filter: events={self.event_filter}, priority={self.priority}")

            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(ws_url, params=params) as ws:
                    logger.info("✓ Connected via WebSocket")
                    self.stats.connection_start = datetime.now()
                    self.reconnect_delay = 1.0  # Reset on successful connection

                    async for msg in ws:
                        if not self.running:
                            break

                        if msg.type == aiohttp.WSMsgType.TEXT:
                            await self.handle_message(json.loads(msg.data))
                        elif msg.type == aiohttp.WSMsgType.ERROR:
                            logger.error(f"WebSocket error: {msg.data}")
                            break
                        elif msg.type in (aiohttp.WSMsgType.CLOSE, aiohttp.WSMsgType.CLOSED):
                            logger.info("WebSocket closed by server")
                            break

                    return True

        except Exception as e:
            logger.error(f"WebSocket connection failed: {e}")
            return False

    async def handle_message(self, message: Dict[str, Any]):
        """Handle incoming WebSocket message"""
        msg_type = message.get('type')

        if msg_type == 'Event':
            event = message.get('event', {})
            await self.handle_event(event)

        elif msg_type == 'Subscribed':
            events = message.get('events', [])
            logger.info(f"Subscribed to events: {', '.join(events)}")

        elif msg_type == 'Error':
            error_msg = message.get('message', 'Unknown error')
            logger.error(f"Server error: {error_msg}")

    async def handle_event(self, event: Dict[str, Any]):
        """Handle incoming event"""
        event_type = event.get('type')
        timestamp = event.get('timestamp')

        self.stats.record_event(event_type)

        # Print event summary
        logger.info(f"[{event_type}] {timestamp}")

        # Handle specific event types
        handlers = {
            'TipsUpdated': self.handle_tips_updated,
            'MessageFinalized': self.handle_message_finalized,
            'MessageAdded': self.handle_message_added,
            'DiversityUpdated': self.handle_diversity_updated,
            'EnergyUpdated': self.handle_energy_updated,
            'KCoreUpdated': self.handle_kcore_updated,
            'AdmissibilityUpdated': self.handle_admissibility_updated,
            'EconomicsUpdated': self.handle_economics_updated,
        }

        handler = handlers.get(event_type)
        if handler:
            await handler(event)

    async def handle_tips_updated(self, event: Dict[str, Any]):
        """Handle TipsUpdated event"""
        count = event.get('count', 0)
        tips = event.get('tips', [])
        logger.info(f"  → Tips count: {count}")
        if tips and len(tips) <= 3:
            for tip in tips:
                logger.info(f"    • {tip[:16]}...")

    async def handle_message_finalized(self, event: Dict[str, Any]):
        """Handle MessageFinalized event"""
        message_id = event.get('message_id', '')
        finality_type = event.get('finality_type', '')
        logger.info(f"  → Message: {message_id[:16]}... ({finality_type})")

    async def handle_message_added(self, event: Dict[str, Any]):
        """Handle MessageAdded event"""
        message_id = event.get('message_id', '')
        depth = event.get('depth', 0)
        logger.info(f"  → Message: {message_id[:16]}... depth={depth}")

    async def handle_diversity_updated(self, event: Dict[str, Any]):
        """Handle DiversityUpdated event"""
        score = event.get('diversity_score', 0.0)
        tips = event.get('total_tips', 0)
        logger.info(f"  → Diversity: {score:.3f}, Tips: {tips}")

    async def handle_energy_updated(self, event: Dict[str, Any]):
        """Handle EnergyUpdated event"""
        active = event.get('active_conflicts', 0)
        resolved = event.get('resolved_conflicts', 0)
        energy = event.get('total_energy', 0.0)
        logger.info(f"  → Active: {active}, Resolved: {resolved}, Energy: {energy:.4f}")

    async def handle_kcore_updated(self, event: Dict[str, Any]):
        """Handle KCoreUpdated event"""
        finalized = event.get('finalized_count', 0)
        pending = event.get('pending_count', 0)
        k_value = event.get('current_k_value')
        logger.info(f"  → Finalized: {finalized}, Pending: {pending}, k={k_value}")

    async def handle_admissibility_updated(self, event: Dict[str, Any]):
        """Handle AdmissibilityUpdated event"""
        overall = event.get('overall_rate', 0.0)
        sample = event.get('sample_size', 0)
        logger.info(f"  → Overall rate: {overall:.2%}, Sample: {sample}")

    async def handle_economics_updated(self, event: Dict[str, Any]):
        """Handle EconomicsUpdated event"""
        circulating = event.get('circulating_supply', '0')
        treasury = event.get('treasury_balance', '0')
        logger.info(f"  → Circulating: {circulating}, Treasury: {treasury}")

    async def start(self):
        """Start the event stream client"""
        self.running = True
        logger.info("Starting ADIC event stream client...")

        try:
            while self.running:
                if await self.connect_websocket():
                    # Connection was successful but closed
                    if self.running:
                        logger.info(f"Reconnecting in {self.reconnect_delay:.1f}s...")
                        await asyncio.sleep(self.reconnect_delay)
                        self.reconnect_delay = min(
                            self.reconnect_delay * 2,
                            self.max_reconnect_delay
                        )
                else:
                    # Connection failed
                    logger.warning(f"Connection failed, retrying in {self.reconnect_delay:.1f}s...")
                    await asyncio.sleep(self.reconnect_delay)
                    self.reconnect_delay = min(
                        self.reconnect_delay * 2,
                        self.max_reconnect_delay
                    )

        except asyncio.CancelledError:
            logger.info("Client cancelled")
        finally:
            self.running = False
            self.stats.print_summary()

    def stop(self):
        """Stop the event stream client"""
        logger.info("Stopping client...")
        self.running = False


async def main():
    """Main entry point"""
    import argparse

    parser = argparse.ArgumentParser(description='ADIC Event Stream Client')
    parser.add_argument(
        '--node-url',
        default='ws://localhost:9121',
        help='Node URL (default: ws://localhost:9121)'
    )
    parser.add_argument(
        '--events',
        default='all',
        help='Event filter (default: all)'
    )
    parser.add_argument(
        '--priority',
        default='all',
        choices=['all', 'high', 'medium', 'low'],
        help='Priority filter (default: all)'
    )

    args = parser.parse_args()

    client = EventStreamClient(
        node_url=args.node_url,
        event_filter=args.events,
        priority=args.priority
    )

    # Handle Ctrl+C gracefully
    def signal_handler(sig, frame):
        logger.info("\nReceived interrupt signal")
        client.stop()

    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)

    # Start client
    await client.start()


if __name__ == '__main__':
    asyncio.run(main())
