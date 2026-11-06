// A page script as a browser would run it: it reads the page's data, fills the
// table, and on a timer adds an island marker for surimono to draw. The timer
// runs on surimono's virtual clock, so the render doesn't wait half a second.
const invoice = JSON.parse(document.getElementById("invoice").textContent);

document.getElementById("title").textContent = `Invoice ${invoice.number}`;

const lines = document.getElementById("lines");
let total = 0;
for (const line of invoice.lines) {
  const amount = line.quantity * line.price;
  total += amount;
  const row = document.createElement("tr");
  for (const value of [line.item, line.quantity, line.price, amount]) {
    const cell = document.createElement("td");
    cell.textContent = value;
    row.append(cell);
  }
  lines.append(row);
}
document.getElementById("total").textContent = `Total: ${total}`;

setTimeout(() => {
  const marker = document.createElement("script");
  marker.type = "application/vnd.surimono+json";
  marker.id = "amounts";
  marker.dataset.renderer = "echarts";
  marker.dataset.width = "480";
  marker.dataset.height = "200";
  marker.textContent = JSON.stringify({
    textStyle: { fontFamily: "Noto Sans" },
    xAxis: { type: "category", data: invoice.lines.map((l) => l.item) },
    yAxis: { type: "value" },
    series: [{ type: "bar", data: invoice.lines.map((l) => l.quantity * l.price) }],
  });
  document.getElementById("chart").append(marker);
}, 500);
