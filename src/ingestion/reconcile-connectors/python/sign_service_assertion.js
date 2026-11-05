const { createPrivateKey, randomUUID, sign } = require('node:crypto');
const { readFileSync } = require('node:fs');

try {
  const key = createPrivateKey(readFileSync(process.env.SERVICE_TOKEN_KEY_FILE));
  if (key.asymmetricKeyType !== 'ec' || key.asymmetricKeyDetails.namedCurve !== 'prime256v1') {
    throw new Error('P-256 key required');
  }
  const now = Math.floor(Date.now() / 1000);
  const header = Buffer.from(JSON.stringify({ alg: 'ES256', typ: 'JWT' })).toString('base64url');
  const claims = Buffer.from(JSON.stringify({
    iss: process.env.SERVICE_TOKEN_NAME,
    sub: process.env.SERVICE_TOKEN_NAME,
    aud: process.env.SERVICE_TOKEN_URL,
    jti: randomUUID(),
    iat: now,
    exp: now + 50,
  })).toString('base64url');
  const input = `${header}.${claims}`;
  const signature = sign('sha256', Buffer.from(input), { key, dsaEncoding: 'ieee-p1363' });
  process.stdout.write(`${input}.${signature.toString('base64url')}`);
} catch {
  process.stderr.write('service assertion signing failed\n');
  process.exitCode = 1;
}
