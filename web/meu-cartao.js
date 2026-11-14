// 2.7.23: clicar na sua foto (barra da esquerda) abre o seu cartão, igual ao Discord.
// Usa o mesmo cartãozinho "névoa" que aparece ao passar o mouse no nome (montarCartao),
// com uma lista simples embaixo: Editar perfil, Status, Mudar nick e Carteira.
// Só mexe na tela: o perfil é salvo pela mesma mensagem "perfil" de sempre.
(() => {
  const $ = (id) => document.getElementById(id);
  const IC = {
    lapis: '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="m4 17.2 10.6-10.6 3 3L7 20.2H4v-3Zm12-12 1.4-1.4a1 1 0 0 1 1.4 0l1.6 1.6a1 1 0 0 1 0 1.4L19 8.2l-3-3Z"/></svg>',
    nick: '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M5 4h14v3h-2V6h-4v12h2v2H9v-2h2V6H7v1H5V4Z"/></svg>',
    lua: '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M12 3a9 9 0 1 0 8.6 11.6A7 7 0 0 1 12.4 3H12Zm6-1 .9 2.1L21 5l-2.1.9L18 8l-.9-2.1L15 5l2.1-.9L18 2Z"/></svg>',
  };

  const caixa = document.createElement("div");
  caixa.id = "meu-cartao";
  caixa.className = "meu-cartao";
  caixa.setAttribute("role", "dialog");
  caixa.setAttribute("aria-label", "Meu perfil");
  caixa.hidden = true;
  document.body.append(caixa);
  let botao = null;

  const aberto = () => !caixa.hidden;
  function fechar() {
    if (!aberto()) return;
    caixa.hidden = true;
    if (botao) botao.setAttribute("aria-expanded", "false");
  }

  function linha(icone, texto, dir) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "mc-op";
    b.innerHTML = icone;
    const t = document.createElement("span"); t.textContent = texto; b.append(t);
    if (dir != null) { const d = document.createElement("span"); d.className = "mc-dir"; d.append(dir); b.append(d); }
    return b;
  }

  // Recado: escreve ali mesmo e aperta Enter. Manda o perfil de sempre, só com o recado novo.
  function salvarRecado(texto) {
    const p = perfilDe(estado.meuId) || {};
    if (texto === recadoDe(p)) return;
    const dados = {
      cor: p.c || "", forca: p.f || 60, degrade: (p.k || p.kg) ? 0 : (p.d || 0), capaY: p.y ?? 50,
      recado: texto, sobre: "", recadoHoras: 0, manterPrazo: false,
    };
    guardar("astra.perfil", JSON.stringify({ ...dados, prazo: 0 }));
    enviar({ t: "perfil", ...dados });
    guardarPerfilDe(estado.meuId, { ...p, r: texto, ra: 0 });   // já aparece; a VPS confirma depois
    mostrarMeuNome();
  }
  function editarRecado(el) {
    if (el.querySelector("input")) return;
    const antes = recadoDe(perfilDe(estado.meuId));
    const i = document.createElement("input");
    i.maxLength = 60; i.placeholder = "Escreva e aperte Enter"; i.autocomplete = "off"; i.value = antes;
    el.textContent = ""; el.append(i); i.focus();
    let feito = false;
    const fim = (salvar) => {
      if (feito) return; feito = true;
      const novo = salvar ? i.value.trim().slice(0, 60) : antes;
      if (salvar) salvarRecado(novo);
      el.classList.toggle("vazio", !novo);
      el.textContent = novo || "+ escrever um recado";
    };
    i.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter") fim(true); if (e.key === "Escape") fim(false); };
    i.onblur = () => fim(true);
  }

  function menuStatus(ancora) {
    const sub = document.createElement("div");
    sub.className = "mc-sub";
    sub.setAttribute("role", "menu");
    for (const [st, info] of Object.entries(STATUS)) {
      const b = document.createElement("button");
      b.type = "button"; b.className = "mc-op" + (st === estado.status ? " on" : "");
      const bol = document.createElement("span"); bol.className = "bolinha st-" + st;
      const t = document.createElement("span"); t.textContent = info.nome;
      if (info.dica) { const s = document.createElement("small"); s.textContent = info.dica; t.append(s); }
      b.append(bol, t);
      b.onclick = (e) => { e.stopPropagation(); mudarStatus(st); desenhar(); };
      sub.append(b);
    }
    // Abre para cima a partir do pé do cartão: a foto fica embaixo na tela, então nunca corta.
    sub.style.bottom = "6px";
    return sub;
  }

  function desenhar() {
    const v = visualDe(estado.meuId, meuNome());
    const c = montarCartao(v);
    caixa.innerHTML = "";
    caixa.style.setProperty("--brilho", v.cor || "#ffffff");

    // Capa: clique troca (abre o editor do perfil já escolhendo o arquivo).
    const capa = c.querySelector(".cp-capa");
    capa.title = "Trocar capa";
    capa.onclick = () => { fechar(); abrirFoto(); $("in-capa").click(); };
    const dica = document.createElement("span"); dica.className = "mc-capa-dica"; dica.textContent = "Trocar capa";

    // Foto: bolinha do status e "Trocar" no hover.
    const av = c.querySelector(".cp-corpo .avatar");
    av.classList.add("mc-foto");
    av.title = "Trocar foto";
    pontoStatus(av, estado.status);
    av.onclick = () => { fechar(); abrirFoto(); $("in-foto").click(); };

    // Recado: sempre aparece (vazio = convite para escrever).
    let rec = c.querySelector(".cp-recado");
    if (!rec) { rec = document.createElement("div"); rec.className = "cp-recado"; c.querySelector(".cp-corpo").append(rec); }
    rec.classList.add("mc-recado");
    if (!v.recado) { rec.classList.add("vazio"); rec.textContent = "+ escrever um recado"; }
    rec.title = "Clique para mudar o recado";
    rec.tabIndex = 0;
    rec.onclick = () => editarRecado(rec);
    rec.onkeydown = (e) => { if (e.key === "Enter") { e.preventDefault(); editarRecado(rec); } };

    const ops = document.createElement("div");
    ops.className = "mc-ops";
    const editar = linha(IC.lapis, "Editar perfil");
    editar.onclick = () => { fechar(); abrirFoto(); };
    const bol = document.createElement("span"); bol.className = "bolinha st-" + (STATUS[estado.status] ? estado.status : "on");
    const st = linha("", (STATUS[estado.status] || STATUS.on).nome, "›");
    st.prepend(bol);
    st.setAttribute("aria-haspopup", "true");
    let sub = null;
    st.onclick = (e) => {
      e.stopPropagation();
      if (sub) { sub.remove(); sub = null; st.classList.remove("on"); return; }
      sub = menuStatus(st); ops.append(sub); st.classList.add("on");
    };
    const nick = linha(IC.nick, "Mudar nick");
    nick.onclick = () => { fechar(); pedirNome(); };
    const saldo = document.createElement("b");
    saldo.className = "mc-doll";
    saldo.textContent = estado.moedas != null ? Number(estado.moedas).toLocaleString("pt-BR") : "...";
    const carteira = linha(IC.lua, "Carteira", saldo);
    carteira.classList.add("mc-info");
    carteira.title = "Seu saldo de AstraDoll";
    carteira.onclick = () => enviar({ t: "carteira" });
    ops.append(editar, st, nick, carteira);

    caixa.append(...c.childNodes, dica, ops);
  }

  function posicionar() {
    if (!botao) return;
    const r = botao.getBoundingClientRect();
    const largura = caixa.offsetWidth, altura = caixa.offsetHeight;
    let x = r.right + 12;
    if (x + largura > innerWidth - 8) x = Math.max(8, innerWidth - largura - 8);
    const y = Math.max(8, Math.min(innerHeight - altura - 8, r.bottom - altura));
    caixa.style.left = x + "px";
    caixa.style.top = y + "px";
  }

  window.abrirMeuCartao = (alvo) => {
    if (aberto()) return fechar();
    if (!estado.meuId) return abrirFoto();   // ainda sem conta: abre o editor de sempre
    botao = alvo;
    desenhar();
    caixa.hidden = false;
    caixa.classList.remove("abrindo"); void caixa.offsetWidth; caixa.classList.add("abrindo");
    posicionar();
    alvo.setAttribute("aria-expanded", "true");
    enviar({ t: "carteira" });   // saldo fresquinho
  };
  // O saldo chegou: atualiza o número sem fechar nada.
  const pintarAntes = window.pintarCarteira;
  if (typeof pintarAntes === "function") {
    window.pintarCarteira = function (...a) {
      pintarAntes.apply(this, a);
      const b = caixa.querySelector(".mc-doll");
      if (b && estado.moedas != null) b.textContent = Number(estado.moedas).toLocaleString("pt-BR");
    };
  }

  document.addEventListener("pointerdown", (e) => {
    if (!aberto() || caixa.contains(e.target) || (botao && botao.contains(e.target))) return;
    fechar();
  }, true);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && aberto()) fechar(); });
  addEventListener("resize", () => { if (aberto()) posicionar(); });
})();
