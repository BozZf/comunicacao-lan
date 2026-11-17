"use strict";
let token = "";
let busy = false;
let generation = 0;
const message = document.getElementById("message");
const station = document.getElementById("station");
const orders = document.getElementById("orders");
const money = cents => (cents / 100).toLocaleString("pt-BR", {style: "currency", currency: "BRL"});
async function api(path, method = "GET") {
  const response = await fetch(path, {
    method, headers: {Authorization: `Bearer ${token}`}, cache: "no-store",
    signal: AbortSignal.timeout(4000)
  });
  if (!response.ok) throw new Error(await response.text());
  return response.status === 204 ? null : response.json();
}
function reset() {
  generation++;
  token = "";
  document.getElementById("token").value = "";
  station.textContent = "—";
  orders.textContent = "—";
  message.textContent = "Desconectado.";
}
async function refresh() {
  if (!token || busy) return;
  busy = true;
  const current = generation;
  try {
    const data = await api("/api/admin/overview");
    if (current !== generation) return;
    const session = data.session;
    station.textContent = `${session.machine_id} · ${session.remaining_seconds ? "Sessão autorizada" : "Sem tempo ativo"} · ${session.remaining_seconds}s restantes · revisão ${session.revision}`;
    orders.replaceChildren();
    if (!data.orders.length) orders.textContent = "Nenhuma cobrança. Escolha um pacote no aplicativo da estação.";
    for (const order of data.orders) {
      const card = document.createElement("article");
      const info = document.createElement("p");
      info.textContent = `${order.id} · ${money(order.amount_cents)} · ${order.duration_seconds}s · ${order.status}`;
      card.append(info);
      if (order.status === "pending") {
        const button = document.createElement("button");
        button.textContent = "Simular pagamento confirmado";
        button.addEventListener("click", async () => {
          const requestGeneration = generation;
          button.disabled = true;
          try {
            await api(`/api/admin/orders/${encodeURIComponent(order.id)}/confirm-demo`, "POST");
            if (requestGeneration !== generation) return;
            message.textContent = "Pagamento simulado confirmado. A estação receberá a autorização na próxima consulta.";
          } catch (error) {
            if (requestGeneration === generation) message.textContent = error.message;
          } finally {
            button.disabled = false;
            if (requestGeneration === generation) await refresh();
          }
        });
        card.append(button);
      }
      orders.append(card);
    }
    message.textContent = "Conectado · atualização automática a cada 2 segundos.";
  } catch (error) {
    if (current === generation) message.textContent = `Falha de comunicação: ${error.message}`;
  } finally { busy = false; }
}
document.getElementById("login").addEventListener("submit", event => {
  event.preventDefault();
  generation++;
  token = document.getElementById("token").value;
  document.getElementById("token").value = "";
  refresh();
});
document.getElementById("logout").addEventListener("click", reset);
window.addEventListener("pagehide", reset);
setInterval(refresh, 2000);
