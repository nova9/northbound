import { check } from 'k6';
import execution from 'k6/execution';
import http from 'k6/http';

const baseUrl = 'https://api.northboundwallet.org';
// The rate is shared by all workers: 50 requests per second total, not per worker.
const requestsPerSecond = Number(__ENV.RATE || 200);
const maximumConcurrentRequests = Number(__ENV.MAX_VUS || 200);

if (!Number.isInteger(requestsPerSecond) || requestsPerSecond < 1) {
  throw new Error('RATE must be a positive integer.');
}
if (!Number.isInteger(maximumConcurrentRequests) || maximumConcurrentRequests < 1) {
  throw new Error('MAX_VUS must be a positive integer.');
}

// Query all three addresses together. Repeatedly fetch the first page only.
const historyRequestBody = JSON.stringify({
  addresses: [
    'north1z5h0t57p3tg2dzgtnfjy45skkttmrw00uygajdcq092ctnpjy5dkstw30q8',
    'north1zxkmn3fflcgj8e77dgeh0j20d85t7gd4vac2e4wtvju8md497kdzql4t8vr',
    'north1z9zngqrm96mu55nvl25zhnc3z3kzdwvu73srdwrrscvgs4rjm2kesa4z854',
  ],
  limit: 100,
  cursor: null,
});
const endpoints = [
  { path: '/v1/chain/tip', name: 'chain_tip' },
  { path: '/v2/fees/estimate', name: 'fees' },
  { path: '/v1/transactions', name: 'history' },
];

export const options = {
  scenarios: {
    read_api: {
      executor: 'constant-arrival-rate',
      rate: requestsPerSecond,
      timeUnit: '1s',
      duration: '10m',
      // Each worker sends one request at a time. Spare workers cover slow requests.
      preAllocatedVUs: maximumConcurrentRequests,
      maxVUs: maximumConcurrentRequests,
      // Allow requests already in flight to finish after the ten minutes end.
      gracefulStop: '3s',
    },
  },
  maxRedirects: 0,
  userAgent: 'Northbound-k6-load-test',
  summaryTrendStats: ['avg', 'med', 'p(95)', 'p(99)', 'max'],
  // These determine the final pass/fail result; they do not abort the load.
  thresholds: {
    http_req_failed: ['rate==0'],
    checks: ['rate==1'],
    dropped_iterations: ['count==0'], // No scheduled requests skipped for lack of workers.
    // For each endpoint, at least 95% of request durations should be under 1,000 ms.
    'http_req_duration{endpoint:chain_tip}': ['p(95)<1000'],
    'http_req_duration{endpoint:fees}': ['p(95)<1000'],
    'http_req_duration{endpoint:history}': ['p(95)<1000'],
  },
};

function fetchAndCheckReadiness() {
  const response = http.get(`${baseUrl}/ready`, {
    timeout: '3s',
    tags: { endpoint: 'readiness' },
  });
  if (response.status !== 200 || response.json('ready') !== true) {
    const details = (response.error || response.body || 'No response body.').slice(0, 500);
    execution.test.abort(`API is not ready: HTTP ${response.status}: ${details}`);
  }
}

export function setup() {
  fetchAndCheckReadiness();
  console.log(`Loading ${baseUrl} at ${requestsPerSecond} requests/s, max ${maximumConcurrentRequests} in flight.`);
}

// k6 calls this once per scheduled iteration. Each call sends exactly one request.
export default function () {
  // Cycle through chain height, fees, then history, sharing the rate equally.
  const endpointIndex = execution.scenario.iterationInTest % endpoints.length;
  const endpoint = endpoints[endpointIndex];
  const path = endpoint.path;
  const parameters = {
    timeout: '3s',
    tags: { endpoint: endpoint.name },
  };
  let response;
  if (endpoint.name === 'history') {
    parameters.headers = { 'Content-Type': 'application/json' };
    response = http.post(`${baseUrl}${path}`, historyRequestBody, parameters);
  } else {
    response = http.get(`${baseUrl}${path}`, parameters);
  }

  if (response.status === 503) {
    // k6 records the HTTP failure automatically. Keep generating load.
    return;
  }

  if (response.status !== 200) {
    const details = (response.error || response.body || 'No response body.').slice(0, 500);
    execution.test.abort(`${path} returned HTTP ${response.status}: ${details}`);
    return;
  }

  let responseBody;
  try {
    responseBody = response.json();
  } catch {
    execution.test.abort(`${path} returned invalid JSON.`);
    return;
  }

  const responseIsValid = check(responseBody, {
    'response has valid fields': (body) => hasValidResponseFields(endpoint.name, body),
  });
  if (!responseIsValid) {
    execution.test.abort(`${path} returned unexpected data.`);
  }
}

export function teardown() {
  fetchAndCheckReadiness();
}

function hasValidResponseFields(endpointName, body) {
  if (!body || typeof body !== 'object') {
    return false;
  }

  if (endpointName === 'chain_tip') {
    return Number.isSafeInteger(body.height) && body.height >= 0;
  }

  if (endpointName === 'history') {
    const hasTransactions = Array.isArray(body.transactions);
    const hasValidCursor = body.next_cursor === null || typeof body.next_cursor === 'string';
    const indexIsReady = body.index?.ready === true;
    return hasTransactions && hasValidCursor && indexIsReady;
  }

  const hasValidFeeRate = Number.isSafeInteger(body.atomic_units_per_kvbyte)
    && body.atomic_units_per_kvbyte >= 0;
  const hasValidBlockTarget = Number.isSafeInteger(body.target_blocks)
    && body.target_blocks > 0;
  return hasValidFeeRate && hasValidBlockTarget;
}
