const REFRESH_MS = 5000;

function shortAddr(addr) {
  if (!addr || addr.length <= 10) return addr;
  return `${addr.slice(0, 4)}…${addr.slice(-4)}`;
}

function shortSig(sig) {
  return `${sig.slice(0, 6)}…${sig.slice(-6)}`;
}

function fmtTime(blockTime) {
  if (blockTime == null) return "—";
  return new Date(blockTime * 1000).toLocaleTimeString();
}

function fmtSol(lamports) {
  return (lamports / 1e9).toFixed(2) + " SOL";
}

function addrCell(addr, label) {
  const text = label ? label : shortAddr(addr);
  return `<a href="https://solscan.io/account/${addr}" target="_blank" rel="noopener"><code class="addr">${text}</code></a>`;
}

function programCell(t) {
  if (!t.program_label) {
    return t.is_inner ? `<span class="badge">CPI transfer</span>` : `<span class="badge">Wallet transfer</span>`;
  }
  return `<span class="badge">${t.program_label}</span>`;
}

function renderRows(tbody, transfers, highlightSigs) {
  tbody.innerHTML = transfers
    .map((t) => {
      const highlight = highlightSigs && highlightSigs.has(t.signature) ? " highlight" : "";
      return `
      <tr class="${highlight}">
        <td>${fmtTime(t.block_time)}</td>
        <td class="amount">${fmtSol(t.lamports)}</td>
        <td>${programCell(t)}</td>
        <td>${addrCell(t.source, t.source_label)}</td>
        <td>${addrCell(t.destination, t.destination_label)}</td>
        <td><a href="https://solscan.io/tx/${t.signature}" target="_blank" rel="noopener">${shortSig(t.signature)}</a></td>
      </tr>`;
    })
    .join("");
}

async function fetchJson(url, options) {
  const res = await fetch(url, options);
  if (!res.ok) throw new Error(`${url} -> ${res.status}`);
  if (res.status === 204) return null;
  return res.json();
}

let chart = null;

function renderChart(buckets) {
  const ctx = document.getElementById("volume-chart");
  const labels = buckets.map((b) => b.hour.slice(11, 16));
  const data = buckets.map((b) => b.sol);

  if (chart) {
    chart.data.labels = labels;
    chart.data.datasets[0].data = data;
    chart.update();
    return;
  }

  chart = new Chart(ctx, {
    type: "bar",
    data: {
      labels,
      datasets: [
        {
          label: "SOL volume",
          data,
          backgroundColor: "#3987e5",
          borderRadius: 4,
          barThickness: 12,
        },
      ],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: { display: false },
        tooltip: {
          callbacks: {
            label: (item) => `${item.parsed.y.toFixed(2)} SOL`,
          },
        },
      },
      scales: {
        x: {
          grid: { display: false },
          ticks: { color: "#898781" },
        },
        y: {
          beginAtZero: true,
          grid: { color: "#2c2c2a" },
          ticks: { color: "#898781" },
        },
      },
    },
  });
}

// --- New chart cards: program breakdown, size distribution, heatmap ---

// Sequential blue ramp (light -> dark), same hue used for the volume chart.
const SEQUENTIAL_BLUE = ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"];

function colorForIntensity(value, max) {
  if (max <= 0) return SEQUENTIAL_BLUE[0];
  const ratio = value / max;
  const idx = Math.min(SEQUENTIAL_BLUE.length - 1, Math.round(ratio * (SEQUENTIAL_BLUE.length - 1)));
  return SEQUENTIAL_BLUE[idx];
}

const CATEGORICAL = ["#3987e5", "#d95926", "#199e70"]; // fixed order: blue, orange, aqua
const OTHER_COLOR = "#52514e";

let programChart = null;
let sizeChart = null;
let heatmapChart = null;

function renderProgramChart(breakdown) {
  const ctx = document.getElementById("program-chart");
  const labels = breakdown.map((b) => b.label);
  const data = breakdown.map((b) => b.sol);
  const colors = breakdown.map((b, i) => (b.label === "Other" ? OTHER_COLOR : CATEGORICAL[i] || OTHER_COLOR));

  if (programChart) {
    programChart.data.labels = labels;
    programChart.data.datasets[0].data = data;
    programChart.data.datasets[0].backgroundColor = colors;
    programChart.update();
    return;
  }

  programChart = new Chart(ctx, {
    type: "doughnut",
    data: { labels, datasets: [{ data, backgroundColor: colors, borderColor: "#1a1a19", borderWidth: 2 }] },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: { position: "bottom", labels: { color: "#c3c2b7", boxWidth: 12, font: { size: 11 } } },
        tooltip: { callbacks: { label: (item) => `${item.label}: ${item.parsed.toFixed(1)} SOL` } },
      },
    },
  });
}

function renderSizeChart(buckets) {
  const ctx = document.getElementById("size-chart");
  const labels = buckets.map((b) => b.range + " SOL");
  const data = buckets.map((b) => b.count);
  const max = Math.max(...data, 1);
  const colors = data.map((v) => colorForIntensity(v, max));

  if (sizeChart) {
    sizeChart.data.labels = labels;
    sizeChart.data.datasets[0].data = data;
    sizeChart.data.datasets[0].backgroundColor = colors;
    sizeChart.update();
    return;
  }

  sizeChart = new Chart(ctx, {
    type: "bar",
    data: { labels, datasets: [{ data, backgroundColor: colors, borderRadius: 4 }] },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: { display: false },
        tooltip: { callbacks: { label: (item) => `${item.parsed.y} transfers` } },
      },
      scales: {
        x: { grid: { display: false }, ticks: { color: "#898781", font: { size: 10 } } },
        y: { beginAtZero: true, grid: { color: "#2c2c2a" }, ticks: { color: "#898781" } },
      },
    },
  });
}

function renderHeatmapChart(hours) {
  const ctx = document.getElementById("heatmap-chart");
  const byHour = new Map(hours.map((h) => [h.hour, h.count]));
  const labels = Array.from({ length: 24 }, (_, h) => String(h).padStart(2, "0"));
  const data = labels.map((_, h) => byHour.get(h) || 0);
  const max = Math.max(...data, 1);
  const colors = data.map((v) => colorForIntensity(v, max));

  if (heatmapChart) {
    heatmapChart.data.datasets[0].data = data;
    heatmapChart.data.datasets[0].backgroundColor = colors;
    heatmapChart.update();
    return;
  }

  heatmapChart = new Chart(ctx, {
    type: "bar",
    data: { labels, datasets: [{ data, backgroundColor: colors, borderRadius: 2 }] },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: { display: false },
        tooltip: { callbacks: { label: (item) => `${item.parsed.y} transfers at ${item.label}:00` } },
      },
      scales: {
        x: { grid: { display: false }, ticks: { color: "#898781", font: { size: 9 }, maxRotation: 0 } },
        y: { beginAtZero: true, grid: { color: "#2c2c2a" }, ticks: { color: "#898781" } },
      },
    },
  });
}

function renderLeaderboard(entries) {
  const tbody = document.getElementById("leaderboard-body");
  tbody.innerHTML = entries
    .map(
      (e, i) => `
      <tr>
        <td>${i + 1}</td>
        <td>${addrCell(e.address, e.label)}</td>
        <td class="amount">${e.sol.toFixed(2)} SOL</td>
        <td>${e.count}</td>
      </tr>`
    )
    .join("");
}

// --- Alerts -----------------------------------------------------------

let seenSignatures = new Set();
let firstRefresh = true;

function showAlertBanner(t) {
  const container = document.getElementById("alert-banners");
  const el = document.createElement("div");
  el.className = "alert-banner";
  el.innerHTML = `
    <span>&#9888; ${fmtSol(t.lamports)} transfer ${t.source_label || shortAddr(t.source)} &rarr; ${
    t.destination_label || shortAddr(t.destination)
  }</span>
    <button aria-label="Dismiss">&times;</button>`;
  el.querySelector("button").addEventListener("click", () => el.remove());
  container.prepend(el);
  setTimeout(() => el.remove(), 15000);
}

function checkAlerts(recent) {
  const threshold = Number(document.getElementById("alert-threshold").value) * 1e9;
  const newHighlights = new Set();

  for (const t of recent) {
    if (seenSignatures.has(t.signature)) continue;
    seenSignatures.add(t.signature);
    if (!firstRefresh && t.lamports >= threshold) {
      showAlertBanner(t);
      newHighlights.add(t.signature);
    }
  }
  firstRefresh = false;
  return newHighlights;
}

// --- Search -------------------------------------------------------------

let activeSearchAddress = null;

document.getElementById("search-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const address = document.getElementById("search-input").value.trim();
  if (!address) return;
  activeSearchAddress = address;
  await refresh();
});

document.getElementById("search-clear").addEventListener("click", () => {
  activeSearchAddress = null;
  document.getElementById("search-input").value = "";
  refresh();
});

// --- Watchlist ------------------------------------------------------------

document.getElementById("watchlist-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const address = document.getElementById("watchlist-address").value.trim();
  const label = document.getElementById("watchlist-label").value.trim();
  if (!address) return;
  await fetchJson("/api/watchlist", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ address, label: label || null }),
  });
  document.getElementById("watchlist-address").value = "";
  document.getElementById("watchlist-label").value = "";
  await refreshWatchlist();
});

async function refreshWatchlist() {
  const entries = await fetchJson("/api/watchlist");
  const ul = document.getElementById("watchlist-entries");
  ul.innerHTML = entries
    .map(
      (w) => `
      <li>
        ${w.label ? `${w.label} (${shortAddr(w.address)})` : shortAddr(w.address)}
        <button data-address="${w.address}" aria-label="Remove">&times;</button>
      </li>`
    )
    .join("");
  ul.querySelectorAll("button").forEach((btn) => {
    btn.addEventListener("click", async () => {
      await fetchJson(`/api/watchlist/${btn.dataset.address}`, { method: "DELETE" });
      await refreshWatchlist();
    });
  });

  const activity = await fetchJson("/api/watchlist/activity?limit=50");
  renderRows(document.getElementById("watchlist-activity-body"), activity);
}

// --- Main refresh loop ------------------------------------------------

async function refresh() {
  try {
    const recentUrl = activeSearchAddress
      ? `/api/transfers/search?address=${encodeURIComponent(activeSearchAddress)}&limit=50`
      : "/api/transfers/recent?limit=50";
    document.getElementById("recent-title").textContent = activeSearchAddress
      ? `Transfers for ${shortAddr(activeSearchAddress)}`
      : "Most recent transfers";

    const [stats, recent, top, timeseries, programBreakdown, sizeDistribution, heatmap, leaderboard] =
      await Promise.all([
        fetchJson("/api/stats"),
        fetchJson(recentUrl),
        fetchJson("/api/transfers/top?limit=10"),
        fetchJson("/api/timeseries?hours=24"),
        fetchJson("/api/breakdown/program"),
        fetchJson("/api/breakdown/size"),
        fetchJson("/api/heatmap"),
        fetchJson("/api/leaderboard?limit=10"),
      ]);

    document.getElementById("stat-total-count").textContent = stats.total_count.toLocaleString();
    document.getElementById("stat-total-sol").textContent = stats.total_sol.toFixed(2) + " SOL";
    document.getElementById("stat-count-24h").textContent = stats.count_24h.toLocaleString();
    document.getElementById("stat-sol-24h").textContent = stats.sol_24h.toFixed(2) + " SOL";

    // Alerts are only meaningful against the live "recent" feed, not a
    // filtered search result.
    const highlights = activeSearchAddress ? new Set() : checkAlerts(recent);

    renderRows(document.getElementById("recent-body"), recent, highlights);
    renderRows(document.getElementById("top-body"), top);
    renderChart(timeseries);
    renderProgramChart(programBreakdown);
    renderSizeChart(sizeDistribution);
    renderHeatmapChart(heatmap);
    renderLeaderboard(leaderboard);
  } catch (e) {
    console.error("dashboard refresh failed:", e);
  }
}

refresh();
refreshWatchlist();
setInterval(refresh, REFRESH_MS);
