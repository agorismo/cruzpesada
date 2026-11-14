// Janelinha do pop-up do chat (popup.html), por cima do jogo. Antes era popup-preload.js
// (Electron); agora usa os eventos do Tauri. O resto da lógica é igual.
const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

const TEMPO = 12000;     // quanto tempo cada mensagem fica na tela
const TEMPO_IMAGEM = 15000;
const MAXIMO = 8;        // linhas ao mesmo tempo
const NOVAS = 3;         // as 3 mais novas ficam bem acesas; as outras apagam um pouco

function iniciais(nome) {
  return (nome || "?").split(/\s+/).map((p) => p[0] || "").join("").slice(0, 2).toUpperCase();
}
const cortar = (t) => (t.length > 220 ? t.slice(0, 220) + "…" : t);
const linkOk = (u) => typeof u === "string" && /^https:\/\/[^\s"'()<>]+$/i.test(u);

function lista() { return document.getElementById("lista"); }

function avisarSeVazio() {
  if (!lista().children.length) invoke("popup_vazio");
}

function tirar(l) {
  if (!l.isConnected || l.classList.contains("saindo")) return;
  l.classList.add("saindo");
  setTimeout(() => { l.remove(); apagarVelhas(); avisarSeVazio(); }, 400);
}

// Não cabe tudo na janelinha: tira as mais antigas (a mais nova fica sempre à vista).
function caber() {
  const ul = lista();
  while (ul.children.length > 1 && ul.scrollHeight > innerHeight) ul.firstElementChild.remove();
}

function apagarVelhas() {
  const ls = [...lista().children].filter((l) => !l.classList.contains("saindo"));
  ls.forEach((l, i) => l.classList.toggle("velha", i < ls.length - NOVAS));
}

function escreverTexto(l, texto) {
  l.querySelector(".txt").textContent = cortar(texto);
}

listen("mensagem", (e) => {
  const m = e.payload;
  const ul = lista();
  // Linha com id: troca o texto da linha que já está na tela.
  const antiga = m.id ? [...ul.children].find((c) => c.dataset.id === m.id) : null;
  if (!m.texto && !m.imagem) { if (antiga) tirar(antiga); return; }
  if (antiga && !antiga.classList.contains("saindo")) {
    escreverTexto(antiga, m.texto);
    clearTimeout(antiga._tempo);
    antiga._tempo = setTimeout(() => tirar(antiga), TEMPO);
    return;
  }
  const l = document.createElement("div");
  l.className = "l" + (m.tipo === "doacao" ? " doacao" : m.tipo === "marcou" ? " marcou" : m.tipo === "jogo" ? " jogo" : "");
  if (m.id) l.dataset.id = m.id;
  const cor = /^#[0-9a-f]{3,8}$/i.test(m.cor || "") ? m.cor : "#5865f2";
  const av = document.createElement("span");
  av.className = "av";
  av.style.backgroundColor = cor;
  if (linkOk(m.foto)) av.style.backgroundImage = `url("${m.foto}")`;
  else av.textContent = iniciais(m.nome);
  const corpo = document.createElement("span");
  corpo.className = "corpo";
  const b = document.createElement("b");
  b.textContent = m.nome;
  if (m.tipo !== "doacao" && m.tipo !== "jogo") b.style.color = cor;
  const t = document.createElement("span");
  t.className = "txt";
  corpo.append(b, t);
  l.append(av, corpo);
  escreverTexto(l, m.texto || "");
  if (linkOk(m.imagem)) {
    const img = document.createElement("img");
    img.alt = "";
    img.src = m.imagem;
    img.onload = () => { caber(); apagarVelhas(); };
    img.onerror = () => { img.remove(); if (!m.texto) t.textContent = m.gif ? "mandou um GIF" : "mandou uma imagem"; };
    corpo.append(img);
  }
  ul.append(l);
  while (ul.children.length > MAXIMO) ul.firstElementChild.remove();
  caber();
  apagarVelhas();
  l._tempo = setTimeout(() => tirar(l), m.imagem ? TEMPO_IMAGEM : TEMPO);
});
