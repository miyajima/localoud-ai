"""Local checkpoint configuration. No downloads or executable model-repository code."""
from dataclasses import dataclass
import json
import os
from pathlib import Path

HERE = Path(__file__).resolve().parent

@dataclass(frozen=True)
class ModelConfig:
    model_id: str
    path: Path
    loader: str
    quantization_bits: int

    @classmethod
    def from_environment(cls):
        config_path = os.environ.get('LOCAL_MODEL_CONFIG')
        if not config_path:
            return cls('abenzerps/Spark-X2.5-4B-MLX-8bit', Path(os.environ.get('SPARK_MODEL_PATH', str(HERE.parent.parent / 'models' / 'Spark-X2.5-4B-MLX-8bit'))), 'spark', 8)
        source = Path(config_path).resolve()
        data = json.loads(source.read_text())
        if set(data) != {'model_id', 'path', 'loader', 'quantization_bits'}:
            raise ValueError('Model config requires model_id, path, loader, quantization_bits')
        if not isinstance(data['model_id'], str) or not data['model_id'].strip() or data['loader'] not in ('spark', 'mlx_lm'):
            raise ValueError('Invalid model ID or loader')
        bits = data['quantization_bits']
        if type(bits) is not int or not 1 <= bits <= 32:
            raise ValueError('quantization_bits must be 1–32')
        path = Path(data['path']).expanduser()
        if not path.is_absolute():
            path = source.parent / path
        return cls(data['model_id'], path.resolve(), data['loader'], bits)

    def verify(self):
        if self.loader == 'spark':
            from download_model import verify
            info = verify(self.path)
            if self.model_id != info['model'] or self.quantization_bits != info['quantization_bits']:
                raise ValueError('Spark configuration does not match verified checkpoint')
            return info
        config = json.loads((self.path / 'config.json').read_text())
        if config.get('quantization', {}).get('bits') != self.quantization_bits:
            raise ValueError('Checkpoint quantization does not match configuration')
        index = self.path / 'model.safetensors.index.json'
        shards = set(json.loads(index.read_text())['weight_map'].values()) if index.exists() else {'model.safetensors'}
        if not shards or any(Path(name).name != name or not (self.path / name).is_file() or (self.path / name).stat().st_size == 0 for name in shards):
            raise ValueError('Missing or invalid checkpoint shards')
        return {'model': self.model_id, 'quantization_bits': self.quantization_bits}
