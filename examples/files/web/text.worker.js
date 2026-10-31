// Same bounded UTF-8 processing contract as model::process on native.
self.onmessage = event => {
  let {text, normalize, trim} = JSON.parse(event.data);
  if (normalize) text = text.replace(/\r\n?/g, '\n');
  if (trim) text = text.split('\n').map(line => line.replace(/[ \t]+$/g, '')).join('\n');
  const words = text.split(/[ \t\r\n\f\v]+/).filter(Boolean).length;
  const statistics = {bytes: new TextEncoder().encode(text).length, characters: [...text].length, words, lines: text.length ? text.split('\n').length : 0};
  self.postMessage(JSON.stringify({text, statistics}));
};
