import { analyzeAll } from './model.js';

self.onmessage = ({ data }) => {
  try {
    self.postMessage({ id: data.id, results: analyzeAll(data.series, data.options) });
  } catch (error) {
    self.postMessage({ id: data.id, error: error.message });
  }
};
