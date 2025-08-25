#!/usr/bin/env python3
"""
ADIC Performance Analysis
Analyze throughput, latency, and finality metrics
"""

import numpy as np
import pandas as pd
import matplotlib.pyplot as plt
from matplotlib.patches import Rectangle
import seaborn as sns
from adic_simulator import AdicSimulator, AdicParams
import time
from typing import List, Dict
import json

class PerformanceAnalyzer:
    """Analyze ADIC protocol performance"""
    
    def __init__(self):
        self.results = []
        sns.set_style("whitegrid")
    
    def benchmark_throughput(self, params: AdicParams, duration: float = 10.0) -> Dict:
        """Benchmark message throughput"""
        simulator = AdicSimulator(params)
        simulator.create_genesis()
        
        start_time = time.time()
        messages_created = 0
        messages_rejected = 0
        
        while time.time() - start_time < duration:
            message = simulator.simulate_step()
            if message:
                messages_created += 1
            else:
                messages_rejected += 1
        
        elapsed = time.time() - start_time
        
        return {
            'duration': elapsed,
            'messages_created': messages_created,
            'messages_rejected': messages_rejected,
            'throughput': messages_created / elapsed,
            'acceptance_rate': messages_created / (messages_created + messages_rejected) if (messages_created + messages_rejected) > 0 else 0,
            'final_tips': len(simulator.tips),
            'finalized': len(simulator.finalized)
        }
    
    def analyze_finality_time(self, params: AdicParams, num_messages: int = 100) -> Dict:
        """Analyze time to finality"""
        simulator = AdicSimulator(params)
        simulator.create_genesis()
        
        finality_times = []
        creation_times = {}
        
        for i in range(num_messages):
            message = simulator.simulate_step()
            if message:
                creation_times[message.id] = time.time()
        
        # Check finality
        for msg_id, create_time in creation_times.items():
            if simulator.check_finality_kcore(msg_id):
                finality_time = time.time() - create_time
                finality_times.append(finality_time)
        
        return {
            'avg_finality_time': np.mean(finality_times) if finality_times else 0,
            'median_finality_time': np.median(finality_times) if finality_times else 0,
            'min_finality_time': np.min(finality_times) if finality_times else 0,
            'max_finality_time': np.max(finality_times) if finality_times else 0,
            'finalization_rate': len(finality_times) / num_messages if num_messages > 0 else 0
        }
    
    def parameter_sweep(self, base_params: AdicParams, param_name: str, 
                       values: List, metric: str = 'throughput') -> pd.DataFrame:
        """Sweep a parameter and measure performance"""
        results = []
        
        for value in values:
            # Create params copy and update
            params = AdicParams(**base_params.__dict__)
            setattr(params, param_name, value)
            
            # Run benchmark
            if metric == 'throughput':
                perf = self.benchmark_throughput(params, duration=5.0)
                results.append({
                    param_name: value,
                    'metric': perf['throughput'],
                    'acceptance_rate': perf['acceptance_rate']
                })
            elif metric == 'finality':
                perf = self.analyze_finality_time(params, num_messages=50)
                results.append({
                    param_name: value,
                    'metric': perf['avg_finality_time'],
                    'finalization_rate': perf['finalization_rate']
                })
        
        return pd.DataFrame(results)
    
    def plot_parameter_impact(self, df: pd.DataFrame, param_name: str, metric_name: str):
        """Plot parameter impact on performance"""
        fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(12, 5))
        
        # Main metric
        ax1.plot(df[param_name], df['metric'], 'b-o', linewidth=2, markersize=8)
        ax1.set_xlabel(param_name)
        ax1.set_ylabel(metric_name)
        ax1.set_title(f'Impact of {param_name} on {metric_name}')
        ax1.grid(True, alpha=0.3)
        
        # Secondary metric
        if 'acceptance_rate' in df.columns:
            ax2.plot(df[param_name], df['acceptance_rate'], 'g-s', linewidth=2, markersize=8)
            ax2.set_ylabel('Acceptance Rate')
        elif 'finalization_rate' in df.columns:
            ax2.plot(df[param_name], df['finalization_rate'], 'r-^', linewidth=2, markersize=8)
            ax2.set_ylabel('Finalization Rate')
        
        ax2.set_xlabel(param_name)
        ax2.set_title(f'Secondary Metrics')
        ax2.grid(True, alpha=0.3)
        
        plt.tight_layout()
        plt.show()
    
    def compare_configurations(self, configs: List[Dict]) -> pd.DataFrame:
        """Compare different parameter configurations"""
        results = []
        
        for config in configs:
            params = AdicParams(**config['params'])
            
            # Run benchmarks
            throughput_perf = self.benchmark_throughput(params, duration=10.0)
            finality_perf = self.analyze_finality_time(params, num_messages=100)
            
            results.append({
                'config': config['name'],
                'throughput': throughput_perf['throughput'],
                'acceptance_rate': throughput_perf['acceptance_rate'],
                'avg_finality_time': finality_perf['avg_finality_time'],
                'finalization_rate': finality_perf['finalization_rate'],
                'final_tips': throughput_perf['final_tips']
            })
        
        return pd.DataFrame(results)
    
    def plot_comparison(self, df: pd.DataFrame):
        """Plot configuration comparison"""
        fig, axes = plt.subplots(2, 2, figsize=(14, 10))
        
        metrics = ['throughput', 'acceptance_rate', 'avg_finality_time', 'finalization_rate']
        titles = ['Throughput (msg/s)', 'Acceptance Rate', 'Avg Finality Time (s)', 'Finalization Rate']
        colors = ['blue', 'green', 'red', 'purple']
        
        for ax, metric, title, color in zip(axes.flat, metrics, titles, colors):
            ax.bar(df['config'], df[metric], color=color, alpha=0.7)
            ax.set_title(title)
            ax.set_xlabel('Configuration')
            ax.set_ylabel(metric)
            ax.tick_params(axis='x', rotation=45)
            
            # Add value labels on bars
            for i, v in enumerate(df[metric]):
                ax.text(i, v, f'{v:.2f}', ha='center', va='bottom')
        
        plt.suptitle('ADIC Protocol Configuration Comparison', fontsize=16)
        plt.tight_layout()
        plt.show()

def main():
    """Run performance analysis"""
    print("ADIC Performance Analysis")
    print("=" * 50)
    
    analyzer = PerformanceAnalyzer()
    
    # Test different configurations
    configs = [
        {
            'name': 'Default',
            'params': {}
        },
        {
            'name': 'High-K',
            'params': {'k': 30}
        },
        {
            'name': 'Low-K',
            'params': {'k': 10}
        },
        {
            'name': 'High-Diversity',
            'params': {'q': 5}
        },
        {
            'name': 'Low-Proximity',
            'params': {'rho': [1, 1, 1]}
        }
    ]
    
    print("\nComparing configurations...")
    comparison_df = analyzer.compare_configurations(configs)
    print("\nResults:")
    print(comparison_df.to_string())
    
    # Plot comparison
    analyzer.plot_comparison(comparison_df)
    
    # Parameter sweep for k
    print("\nAnalyzing k-parameter impact...")
    base_params = AdicParams()
    k_values = [5, 10, 15, 20, 25, 30, 35, 40]
    
    k_impact = analyzer.parameter_sweep(base_params, 'k', k_values, metric='throughput')
    analyzer.plot_parameter_impact(k_impact, 'k', 'Throughput (msg/s)')
    
    # Parameter sweep for q
    print("\nAnalyzing q-parameter impact...")
    q_values = [1, 2, 3, 4, 5, 6]
    
    q_impact = analyzer.parameter_sweep(base_params, 'q', q_values, metric='throughput')
    analyzer.plot_parameter_impact(q_impact, 'q', 'Throughput (msg/s)')
    
    # Save results
    results = {
        'comparison': comparison_df.to_dict(),
        'k_impact': k_impact.to_dict(),
        'q_impact': q_impact.to_dict()
    }
    
    with open('performance_results.json', 'w') as f:
        json.dump(results, f, indent=2, default=str)
    
    print("\nResults saved to performance_results.json")

if __name__ == "__main__":
    main()