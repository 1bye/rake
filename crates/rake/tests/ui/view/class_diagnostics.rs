fn main() {
    let _ = rake::view! { <div class="flex justify-betwen hovr:p-2 p-nope opacity-[2] font-[] text-[#nope]"/> };
    let _ = rake::view! { <div class={if true { "p-nope gap-nope" } else { "w-nope h-nope" }}/> };
    let _ = rake::view! { <p><span class="font-[] text-[#badhex] w-nope">bad</span></p> };
}
