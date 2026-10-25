import { spawn } from 'node:child_process';

// All runners use the same timeout, output bound, exit checks, and pipe draining.
export async function execute(file, args, { cwd, env, timeout = 30_000 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(file, args, { cwd, env, shell: false, stdio: ['ignore', 'pipe', 'pipe'] });
    const stdout = [];
    const stderr = [];
    let bytes = 0;
    let timedOut = false;
    let tooLarge = false;
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, timeout);
    for (const [stream, buffers] of [[child.stdout, stdout], [child.stderr, stderr]]) {
      stream.on('data', chunk => {
        bytes += chunk.length;
        if (bytes > 16 * 1024 * 1024) { tooLarge = true; child.kill('SIGKILL'); }
        else buffers.push(chunk);
      });
    }
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      if (code !== 0 || timedOut || tooLarge) {
        reject(new Error(`${file} ${args.join(' ')} failed (${code ?? signal}${timedOut ? ', timeout' : ''}${tooLarge ? ', output limit' : ''})\n${Buffer.concat(stderr)}`));
      } else {
        resolve({ stdout: Buffer.concat(stdout), stderr: Buffer.concat(stderr) });
      }
    });
  });
}
