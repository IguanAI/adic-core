#!/usr/bin/env python3
"""
Enhanced ADIC Protocol Simulator with libadic support
Uses libadic for accurate p-adic arithmetic when available
"""

import numpy as np
import networkx as nx
import matplotlib.pyplot as plt
from dataclasses import dataclass, field
from typing import List, Dict, Set, Tuple, Optional
import hashlib
import time

# Try to import libadic for accurate p-adic arithmetic
try:
    import libadic
    LIBADIC_AVAILABLE = True
    print("Using libadic for p-adic arithmetic")
except ImportError:
    LIBADIC_AVAILABLE = False
    print("libadic not available, using fallback implementation")
    
from adic_simulator import AdicParams, Message

class PadicNumber:
    """Wrapper for p-adic numbers using libadic or fallback"""
    
    def __init__(self, value: int, p: int, precision: int = 10):
        self.p = p
        self.precision = precision
        
        if LIBADIC_AVAILABLE:
            # Use libadic for accurate computation
            self.qp = libadic.Qp(p, precision)
            self.number = self.qp(value)
        else:
            # Fallback to simple implementation
            self.digits = []
            v = value
            for _ in range(precision):
                self.digits.append(v % p)
                v //= p
                if v == 0:
                    break
            while len(self.digits) < precision:
                self.digits.append(0)
    
    def valuation(self, other: 'PadicNumber') -> int:
        """Compute p-adic valuation of difference"""
        if LIBADIC_AVAILABLE:
            diff = self.number - other.number
            return diff.valuation() if diff != 0 else self.precision
        else:
            # Fallback implementation
            for i in range(min(len(self.digits), len(other.digits))):
                if self.digits[i] != other.digits[i]:
                    return i
            return min(len(self.digits), len(other.digits))
    
    def distance(self, other: 'PadicNumber') -> float:
        """Compute p-adic distance"""
        if LIBADIC_AVAILABLE:
            return float((self.number - other.number).norm())
        else:
            v = self.valuation(other)
            return self.p ** (-v)
    
    def ball_id(self, radius: int) -> tuple:
        """Get ball identifier at given radius"""
        if LIBADIC_AVAILABLE:
            # Extract first 'radius' digits from libadic representation
            str_rep = str(self.number)
            digits = []
            for char in str_rep:
                if char.isdigit():
                    digits.append(int(char))
                    if len(digits) >= radius:
                        break
            return tuple(digits[:radius])
        else:
            return tuple(self.digits[:radius])
    
    def __str__(self):
        if LIBADIC_AVAILABLE:
            return str(self.number)
        else:
            return f"Qp({self.digits[:5]}...)"

class EnhancedAdicSimulator:
    """Enhanced ADIC simulator with libadic support"""
    
    def __init__(self, params: AdicParams):
        self.params = params
        self.messages: Dict[str, Message] = {}
        self.dag = nx.DiGraph()
        self.tips: Set[str] = set()
        self.finalized: Set[str] = set()
        self.message_counter = 0
        self.reputation_scores: Dict[str, float] = {}
        
        # Initialize p-adic field if libadic is available
        if LIBADIC_AVAILABLE:
            self.qp_fields = [libadic.Qp(params.p, 20) for _ in range(len(params.rho))]
    
    def generate_message_id(self) -> str:
        """Generate unique message ID"""
        self.message_counter += 1
        return hashlib.sha256(f"msg_{self.message_counter}_{time.time()}".encode()).hexdigest()[:16]
    
    def compute_proximity_matrix(self, tips: List[str], new_features: List[PadicNumber]) -> np.ndarray:
        """Compute proximity matrix between tips and new message"""
        n_tips = len(tips)
        n_axes = len(new_features)
        proximity_matrix = np.zeros((n_tips, n_axes))
        
        for i, tip_id in enumerate(tips):
            tip = self.messages[tip_id]
            for j, (tip_feat, new_feat) in enumerate(zip(tip.features, new_features)):
                proximity_matrix[i, j] = new_feat.distance(tip_feat)
        
        return proximity_matrix
    
    def select_parents_enhanced(self, new_features: List[PadicNumber], author: str) -> List[str]:
        """Enhanced parent selection using accurate p-adic distances"""
        if not self.tips:
            return []
        
        tips = list(self.tips)
        if len(tips) <= self.params.d + 1:
            return tips[:self.params.d + 1]
        
        # Compute proximity matrix
        proximity_matrix = self.compute_proximity_matrix(tips, new_features)
        
        # Calculate weights incorporating p-adic distances
        weights = []
        for i, tip_id in enumerate(tips):
            tip = self.messages[tip_id]
            
            # Trust weight
            trust = np.log(1 + tip.reputation)
            
            # Proximity weight (product across axes)
            proximity = np.prod(np.exp(-proximity_matrix[i]))
            
            # Depth weight
            depth_weight = 1.0 + tip.depth / 100.0
            
            # Combined weight
            weight = trust * proximity * depth_weight * self.params.lambda_w
            weights.append(weight)
        
        # Normalize and select
        weights = np.array(weights)
        weights = weights / weights.sum() if weights.sum() > 0 else np.ones(len(tips)) / len(tips)
        
        # Ensure diversity by selecting from different balls
        selected = []
        balls_per_axis = [set() for _ in range(len(self.params.rho))]
        
        # Sort tips by weight
        sorted_indices = np.argsort(weights)[::-1]
        
        for idx in sorted_indices:
            tip_id = tips[idx]
            tip = self.messages[tip_id]
            
            # Check if this tip adds diversity
            adds_diversity = False
            for j, (feat, rho) in enumerate(zip(tip.features, self.params.rho)):
                ball = feat.ball_id(rho)
                if ball not in balls_per_axis[j]:
                    adds_diversity = True
                    balls_per_axis[j].add(ball)
            
            if adds_diversity or len(selected) < self.params.q:
                selected.append(tip_id)
                
            if len(selected) >= self.params.d + 1:
                break
        
        # Fill remaining slots if needed
        while len(selected) < min(self.params.d + 1, len(tips)):
            remaining = [t for t in tips if t not in selected]
            if remaining:
                idx = np.random.choice(len(remaining))
                selected.append(remaining[idx])
            else:
                break
        
        return selected
    
    def check_admissibility_enhanced(self, message: Message) -> Tuple[bool, Dict]:
        """Enhanced admissibility check with detailed metrics"""
        if len(message.parents) < self.params.d + 1:
            return True, {"status": "bootstrap", "c1": True, "c2": True, "c3": True}
        
        parents = [self.messages[p] for p in message.parents if p in self.messages]
        metrics = {"c1": True, "c2": True, "c3": True, "details": []}
        
        # C1: Proximity constraint with exact p-adic distance
        for i, rho_i in enumerate(self.params.rho):
            distances = []
            for parent in parents:
                v = message.features[i].valuation(parent.features[i])
                distances.append(v)
                if v < rho_i:
                    metrics["c1"] = False
                    metrics["details"].append(f"C1 violation: axis {i}, v={v} < ρ={rho_i}")
            metrics[f"axis_{i}_min_valuation"] = min(distances) if distances else 0
        
        # C2: Diversity constraint with ball counting
        for i in range(len(self.params.rho)):
            balls = set()
            for parent in parents:
                ball = parent.features[i].ball_id(self.params.rho[i])
                balls.add(ball)
            
            metrics[f"axis_{i}_balls"] = len(balls)
            if len(balls) < self.params.q:
                metrics["c2"] = False
                metrics["details"].append(f"C2 violation: axis {i}, balls={len(balls)} < q={self.params.q}")
        
        # C3: Reputation constraint
        reputations = [p.reputation for p in parents]
        total_rep = sum(reputations)
        min_rep = min(reputations) if reputations else 0
        
        metrics["total_reputation"] = total_rep
        metrics["min_reputation"] = min_rep
        
        if total_rep < self.params.r_sum_min:
            metrics["c3"] = False
            metrics["details"].append(f"C3 violation: total_rep={total_rep:.2f} < {self.params.r_sum_min}")
        if min_rep < self.params.r_min:
            metrics["c3"] = False
            metrics["details"].append(f"C3 violation: min_rep={min_rep:.2f} < {self.params.r_min}")
        
        is_admissible = metrics["c1"] and metrics["c2"] and metrics["c3"]
        metrics["status"] = "admissible" if is_admissible else "rejected"
        
        return is_admissible, metrics
    
    def analyze_dag_structure(self) -> Dict:
        """Analyze DAG structure with graph metrics"""
        metrics = {
            'nodes': len(self.dag.nodes),
            'edges': len(self.dag.edges),
            'tips': len(self.tips),
            'finalized': len(self.finalized),
            'avg_degree': 0,
            'max_degree': 0,
            'diameter': 0,
            'avg_clustering': 0,
            'components': 0
        }
        
        if self.dag.nodes:
            degrees = dict(self.dag.degree())
            metrics['avg_degree'] = np.mean(list(degrees.values()))
            metrics['max_degree'] = max(degrees.values())
            
            # Convert to undirected for some metrics
            undirected = self.dag.to_undirected()
            metrics['avg_clustering'] = nx.average_clustering(undirected)
            metrics['components'] = nx.number_connected_components(undirected)
            
            # DAG-specific metrics
            if nx.is_directed_acyclic_graph(self.dag):
                metrics['diameter'] = nx.dag_longest_path_length(self.dag)
        
        return metrics

def run_comparison():
    """Compare simulator with and without libadic"""
    params = AdicParams()
    
    print("Running comparison between implementations...")
    print("=" * 50)
    
    # Test with enhanced simulator
    enhanced_sim = EnhancedAdicSimulator(params)
    
    # Create test p-adic numbers
    p = 3
    values = [10, 20, 30, 11, 21, 31]
    
    print(f"\nP-adic numbers (p={p}):")
    padic_nums = []
    for v in values:
        pn = PadicNumber(v, p)
        padic_nums.append(pn)
        print(f"  {v} -> {pn}")
    
    print("\nDistance matrix:")
    for i in range(len(padic_nums)):
        distances = []
        for j in range(len(padic_nums)):
            d = padic_nums[i].distance(padic_nums[j])
            distances.append(f"{d:.4f}")
        print(f"  {values[i]:3d}: {' '.join(distances)}")
    
    print("\nBall assignments (radius=2):")
    balls = {}
    for v, pn in zip(values, padic_nums):
        ball = pn.ball_id(2)
        if ball not in balls:
            balls[ball] = []
        balls[ball].append(v)
    
    for ball, members in sorted(balls.items()):
        print(f"  Ball {ball}: {members}")

def main():
    """Run enhanced simulation"""
    print("Enhanced ADIC Protocol Simulator")
    print(f"libadic available: {LIBADIC_AVAILABLE}")
    print("=" * 50)
    
    # Run comparison
    run_comparison()

if __name__ == "__main__":
    main()