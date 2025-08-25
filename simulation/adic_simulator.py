#!/usr/bin/env python3
"""
ADIC Protocol Simulator
A Python framework for simulating and visualizing the ADIC DAG consensus protocol
"""

import numpy as np
import networkx as nx
import matplotlib.pyplot as plt
from dataclasses import dataclass, field
from typing import List, Dict, Set, Tuple, Optional
import hashlib
import json
import random
from collections import defaultdict, deque
import time

@dataclass
class AdicParams:
    """Protocol parameters"""
    p: int = 3  # Prime for p-adic system
    d: int = 3  # Degree (number of parents - 1)
    rho: List[int] = field(default_factory=lambda: [2, 2, 1])  # Proximity thresholds
    q: int = 3  # Minimum diversity per axis
    k: int = 20  # K-core parameter
    depth_star: int = 100  # Minimum depth for finality
    delta: int = 5  # Depth difference threshold
    r_min: float = 0.1  # Minimum individual reputation
    r_sum_min: float = 10.0  # Minimum total reputation
    lambda_w: float = 0.5  # MRW weight parameter
    beta: float = 0.1  # Trust decay factor
    mu: float = 0.05  # Conflict penalty factor

class QpDigits:
    """P-adic number representation"""
    def __init__(self, value: int, p: int, precision: int = 10):
        self.p = p
        self.precision = precision
        self.digits = []
        
        v = value
        for _ in range(precision):
            self.digits.append(v % p)
            v //= p
            if v == 0:
                break
        
        # Pad with zeros
        while len(self.digits) < precision:
            self.digits.append(0)
    
    def ball_id(self, radius: int) -> tuple:
        """Get ball identifier at given radius"""
        return tuple(self.digits[:radius])
    
    def __str__(self):
        return f"Qp({self.digits[:5]}...)"

def vp_diff(x: QpDigits, y: QpDigits) -> int:
    """Compute p-adic valuation of difference"""
    assert x.p == y.p, "Different primes"
    
    for i in range(min(len(x.digits), len(y.digits))):
        if x.digits[i] != y.digits[i]:
            return i
    return min(len(x.digits), len(y.digits))

def proximity_score(x: QpDigits, y: QpDigits, p: int) -> float:
    """Compute proximity score between two p-adic numbers"""
    v = vp_diff(x, y)
    return p ** (-v)

@dataclass
class Message:
    """ADIC message"""
    id: str
    parents: List[str]
    features: List[QpDigits]  # One per axis
    timestamp: float
    author: str
    content: str
    depth: int = 0
    reputation: float = 1.0
    finalized: bool = False
    
    def __hash__(self):
        return hash(self.id)

class AdicSimulator:
    """Main ADIC protocol simulator"""
    
    def __init__(self, params: AdicParams):
        self.params = params
        self.messages: Dict[str, Message] = {}
        self.dag = nx.DiGraph()
        self.tips: Set[str] = set()
        self.finalized: Set[str] = set()
        self.message_counter = 0
        self.reputation_scores: Dict[str, float] = defaultdict(lambda: 1.0)
        
    def generate_message_id(self) -> str:
        """Generate unique message ID"""
        self.message_counter += 1
        return hashlib.sha256(f"msg_{self.message_counter}_{time.time()}".encode()).hexdigest()[:16]
    
    def create_genesis(self) -> Message:
        """Create genesis message"""
        msg_id = self.generate_message_id()
        features = [QpDigits(0, self.params.p) for _ in range(len(self.params.rho))]
        
        genesis = Message(
            id=msg_id,
            parents=[],
            features=features,
            timestamp=time.time(),
            author="genesis",
            content="Genesis message",
            depth=0,
            reputation=10.0
        )
        
        self.add_message(genesis)
        return genesis
    
    def add_message(self, message: Message):
        """Add message to the DAG"""
        self.messages[message.id] = message
        self.dag.add_node(message.id, message=message)
        
        # Update DAG edges
        for parent_id in message.parents:
            if parent_id in self.messages:
                self.dag.add_edge(parent_id, message.id)
        
        # Update tips
        self.tips.discard(*message.parents)
        self.tips.add(message.id)
        
        # Calculate depth
        if message.parents:
            parent_depths = [self.messages[p].depth for p in message.parents if p in self.messages]
            message.depth = max(parent_depths) + 1 if parent_depths else 0
    
    def select_parents_mrw(self, new_features: List[QpDigits], author: str) -> List[str]:
        """Select parents using Multi-axis Random Walk"""
        if not self.tips:
            return []
        
        tips = list(self.tips)
        if len(tips) <= self.params.d + 1:
            return tips[:self.params.d + 1]
        
        # Calculate weights for each tip
        weights = []
        for tip_id in tips:
            tip = self.messages[tip_id]
            
            # Trust weight
            trust = np.log(1 + tip.reputation)
            
            # Proximity weight
            proximity = 1.0
            for i, (tip_feat, new_feat) in enumerate(zip(tip.features, new_features)):
                prox = proximity_score(tip_feat, new_feat, self.params.p)
                proximity *= prox
            
            # Depth weight (prefer deeper messages)
            depth_weight = 1.0 + tip.depth / 100.0
            
            # Combined weight
            weight = trust * proximity * depth_weight * self.params.lambda_w
            weights.append(weight)
        
        # Normalize weights
        total_weight = sum(weights)
        if total_weight > 0:
            weights = [w / total_weight for w in weights]
        else:
            weights = [1.0 / len(tips) for _ in tips]
        
        # Select parents
        selected = np.random.choice(tips, size=min(self.params.d + 1, len(tips)), 
                                  replace=False, p=weights)
        return list(selected)
    
    def check_admissibility(self, message: Message) -> Tuple[bool, str]:
        """Check C1-C3 admissibility constraints"""
        if len(message.parents) < self.params.d + 1:
            return True, "Bootstrap phase"
        
        parents = [self.messages[p] for p in message.parents if p in self.messages]
        
        # C1: Proximity constraint
        for i, rho_i in enumerate(self.params.rho):
            for parent in parents:
                v = vp_diff(message.features[i], parent.features[i])
                if v < rho_i:
                    return False, f"C1 failed: proximity violation on axis {i}"
        
        # C2: Diversity constraint
        for i in range(len(self.params.rho)):
            balls = set()
            for parent in parents:
                ball = parent.features[i].ball_id(self.params.rho[i])
                balls.add(ball)
            if len(balls) < self.params.q:
                return False, f"C2 failed: insufficient diversity on axis {i}"
        
        # C3: Reputation constraint
        total_rep = sum(p.reputation for p in parents)
        min_rep = min(p.reputation for p in parents)
        
        if total_rep < self.params.r_sum_min:
            return False, f"C3 failed: insufficient total reputation {total_rep}"
        if min_rep < self.params.r_min:
            return False, f"C3 failed: parent with low reputation {min_rep}"
        
        return True, "Admissible"
    
    def check_finality_kcore(self, message_id: str) -> bool:
        """Check if message is in k-core for finality"""
        if message_id in self.finalized:
            return True
        
        # Build subgraph of ancestors
        ancestors = nx.ancestors(self.dag, message_id)
        ancestors.add(message_id)
        subgraph = self.dag.subgraph(ancestors)
        
        # Compute k-core
        k_core = nx.k_core(subgraph.to_undirected(), k=self.params.k)
        
        if message_id in k_core:
            # Check depth from tips
            message = self.messages[message_id]
            tip_depths = [self.messages[t].depth for t in self.tips]
            if tip_depths:
                max_tip_depth = max(tip_depths)
                if max_tip_depth - message.depth >= self.params.depth_star:
                    return True
        
        return False
    
    def simulate_step(self, author: str = None) -> Optional[Message]:
        """Simulate one step of the protocol"""
        if author is None:
            author = f"node_{random.randint(1, 10)}"
        
        # Generate random features
        features = []
        for i in range(len(self.params.rho)):
            value = random.randint(0, 1000)
            features.append(QpDigits(value, self.params.p))
        
        # Select parents
        parents = self.select_parents_mrw(features, author)
        
        # Create message
        msg_id = self.generate_message_id()
        message = Message(
            id=msg_id,
            parents=parents,
            features=features,
            timestamp=time.time(),
            author=author,
            content=f"Message from {author}",
            reputation=self.reputation_scores[author]
        )
        
        # Check admissibility
        admissible, reason = self.check_admissibility(message)
        if not admissible:
            print(f"Message rejected: {reason}")
            return None
        
        # Add to DAG
        self.add_message(message)
        
        # Update reputation (simplified)
        self.reputation_scores[author] += 0.1
        
        # Check finality for recent messages
        for msg_id in list(self.messages.keys())[-100:]:
            if self.check_finality_kcore(msg_id):
                self.finalized.add(msg_id)
                self.messages[msg_id].finalized = True
        
        return message
    
    def simulate(self, steps: int, verbose: bool = False):
        """Run simulation for given number of steps"""
        # Create genesis
        self.create_genesis()
        
        created = 0
        rejected = 0
        
        for step in range(steps):
            message = self.simulate_step()
            if message:
                created += 1
                if verbose and step % 10 == 0:
                    print(f"Step {step}: Created message {message.id[:8]}... "
                          f"(depth={message.depth}, tips={len(self.tips)})")
            else:
                rejected += 1
        
        print(f"\nSimulation complete:")
        print(f"  Messages created: {created}")
        print(f"  Messages rejected: {rejected}")
        print(f"  Total messages: {len(self.messages)}")
        print(f"  Current tips: {len(self.tips)}")
        print(f"  Finalized: {len(self.finalized)}")
    
    def visualize_dag(self, max_nodes: int = 100):
        """Visualize the DAG structure"""
        plt.figure(figsize=(15, 10))
        
        # Select subset if too many nodes
        if len(self.dag.nodes) > max_nodes:
            nodes = list(self.dag.nodes)[-max_nodes:]
            subgraph = self.dag.subgraph(nodes)
        else:
            subgraph = self.dag
        
        # Calculate layout
        pos = nx.spring_layout(subgraph, k=2, iterations=50)
        
        # Color nodes by status
        node_colors = []
        for node in subgraph.nodes:
            if node in self.finalized:
                node_colors.append('green')
            elif node in self.tips:
                node_colors.append('red')
            else:
                node_colors.append('lightblue')
        
        # Draw
        nx.draw(subgraph, pos, 
                node_color=node_colors,
                node_size=300,
                with_labels=False,
                arrows=True,
                edge_color='gray',
                alpha=0.7)
        
        # Add legend
        from matplotlib.patches import Patch
        legend_elements = [
            Patch(facecolor='green', label='Finalized'),
            Patch(facecolor='red', label='Tips'),
            Patch(facecolor='lightblue', label='Pending')
        ]
        plt.legend(handles=legend_elements, loc='upper left')
        
        plt.title(f"ADIC DAG Structure (showing {len(subgraph.nodes)} of {len(self.dag.nodes)} nodes)")
        plt.axis('off')
        plt.tight_layout()
        plt.show()
    
    def analyze_metrics(self) -> Dict:
        """Analyze simulation metrics"""
        metrics = {
            'total_messages': len(self.messages),
            'tips': len(self.tips),
            'finalized': len(self.finalized),
            'finalization_rate': len(self.finalized) / len(self.messages) if self.messages else 0,
            'avg_depth': np.mean([m.depth for m in self.messages.values()]) if self.messages else 0,
            'max_depth': max([m.depth for m in self.messages.values()]) if self.messages else 0,
            'avg_parents': np.mean([len(m.parents) for m in self.messages.values()]) if self.messages else 0,
            'connected_components': nx.number_weakly_connected_components(self.dag),
            'dag_diameter': nx.dag_longest_path_length(self.dag) if self.dag.nodes else 0,
        }
        
        # Degree distribution
        degrees = dict(self.dag.degree())
        if degrees:
            metrics['avg_degree'] = np.mean(list(degrees.values()))
            metrics['max_degree'] = max(degrees.values())
        
        return metrics

def main():
    """Run example simulation"""
    print("ADIC Protocol Simulator")
    print("=" * 50)
    
    # Initialize with default parameters
    params = AdicParams()
    simulator = AdicSimulator(params)
    
    # Run simulation
    print("\nRunning simulation...")
    simulator.simulate(steps=100, verbose=True)
    
    # Analyze metrics
    print("\nMetrics:")
    metrics = simulator.analyze_metrics()
    for key, value in metrics.items():
        print(f"  {key}: {value:.2f}" if isinstance(value, float) else f"  {key}: {value}")
    
    # Visualize
    print("\nGenerating visualization...")
    simulator.visualize_dag(max_nodes=50)

if __name__ == "__main__":
    main()