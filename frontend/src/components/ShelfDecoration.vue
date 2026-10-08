<script setup lang="ts">
import { useId } from "vue";

defineProps<{
  kind: "plant" | "vase" | "candle" | "frame";
  color: string;
}>();

const id = `shelf-decoration-${useId()}`;
const paint = (name: string) => `url(#${id}-${name})`;
</script>

<template>
  <svg class="shelf-decoration-art" :style="{ color }" viewBox="0 0 100 160"
    preserveAspectRatio="xMidYMax meet" aria-hidden="true" focusable="false">
    <defs>
      <linearGradient :id="`${id}-surface`" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0" stop-color="#231911" stop-opacity=".29" />
        <stop offset=".22" stop-color="#ffffff" stop-opacity=".09" />
        <stop offset=".4" stop-color="#ffffff" stop-opacity=".2" />
        <stop offset=".73" stop-color="#ffffff" stop-opacity="0" />
        <stop offset="1" stop-color="#271b15" stop-opacity=".23" />
      </linearGradient>
      <linearGradient :id="`${id}-leaf`" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#658067" />
        <stop offset=".5" stop-color="#3e6149" />
        <stop offset="1" stop-color="#294833" />
      </linearGradient>
      <linearGradient :id="`${id}-leaf-light`" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#8b9c74" />
        <stop offset="1" stop-color="#4f7050" />
      </linearGradient>
      <linearGradient :id="`${id}-brass`" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0" stop-color="#6f5432" />
        <stop offset=".3" stop-color="#bd9d65" />
        <stop offset=".47" stop-color="#d7bd89" />
        <stop offset=".75" stop-color="#a48451" />
        <stop offset="1" stop-color="#80603b" />
      </linearGradient>
      <linearGradient :id="`${id}-paper`" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#efe9da" />
        <stop offset="1" stop-color="#dcd1bc" />
      </linearGradient>
      <linearGradient :id="`${id}-flame`" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#e9af65" stop-opacity=".75" />
        <stop offset=".7" stop-color="#f7d49a" />
        <stop offset="1" stop-color="#e8aa63" />
      </linearGradient>
      <radialGradient :id="`${id}-glow`">
        <stop offset="0" stop-color="#f4cb86" stop-opacity=".21" />
        <stop offset="1" stop-color="#f4cb86" stop-opacity="0" />
      </radialGradient>
      <clipPath :id="`${id}-picture`"><rect x="23" y="66" width="55" height="77" /></clipPath>
    </defs>

    <g v-if="kind === 'plant'">
      <ellipse cx="51" cy="157" rx="30" ry="2" fill="#211910" opacity=".14" />
      <g fill="none" stroke="#4b6246" stroke-linecap="round">
        <path d="M50 124 C46 98 47 59 53 28 M48 90 Q31 80 24 58 M49 103 Q67 87 76 57 M50 69 Q33 53 29 32 M51 54 Q64 47 67 29 M47 111 Q29 105 18 86 M52 116 Q69 103 83 89" stroke-width="1.8" />
        <path d="M48 82 Q66 67 78 42 M47 100 Q29 91 22 71" stroke-width="1.25" />
      </g>
      <g :fill="paint('leaf')">
        <path d="M53 39 C40 28 43 14 57 8 C63 19 63 31 53 39Z" />
        <path d="M30 49 C13 47 10 32 13 24 C26 26 34 36 30 49Z" />
        <path d="M29 73 C11 72 7 56 10 47 C24 51 32 60 29 73Z" />
        <path d="M39 97 C24 98 15 87 15 77 C30 77 39 85 39 97Z" />
        <path d="M29 112 C13 117 3 106 5 95 C18 94 28 100 29 112Z" />
        <path d="M57 60 C57 43 68 36 80 36 C80 49 71 59 57 60Z" />
        <path d="M60 86 C63 68 77 63 89 68 C85 81 74 89 60 86Z" />
        <path d="M65 106 C73 91 87 90 95 98 C87 109 76 114 65 106Z" />
      </g>
      <g :fill="paint('leaf-light')">
        <path d="M61 42 C57 29 66 18 77 16 C78 29 73 39 61 42Z" />
        <path d="M63 70 C67 54 81 49 93 53 C90 66 78 74 63 70Z" />
        <path d="M48 70 C33 67 27 57 30 44 C43 47 51 56 48 70Z" />
        <path d="M48 100 C34 97 28 89 30 78 C43 79 51 87 48 100Z" />
        <path d="M51 123 C53 109 63 102 74 104 C73 116 65 123 51 123Z" />
      </g>
      <g fill="none" stroke="#b6c3a0" stroke-width=".65" opacity=".34">
        <path d="M53 35 L56 14 M28 45 L16 29 M26 67 L13 53 M64 82 L84 72 M63 65 L87 56 M58 55 L74 40 M61 36 L73 21 M43 65 L33 49 M44 95 L33 82" />
      </g>
      <ellipse cx="50" cy="121" rx="25" ry="5" fill="#392c22" />
      <path d="M26 120 L31 150 Q50 161 69 150 L74 120Z" fill="currentColor" />
      <path d="M26 120 L31 150 Q50 161 69 150 L74 120Z" :fill="paint('surface')" />
      <path d="M25 117 Q50 124 75 117 L74 125 Q50 132 26 125Z" fill="currentColor" />
      <path d="M25 117 Q50 124 75 117 L74 125 Q50 132 26 125Z" :fill="paint('surface')" />
      <path d="M28 118 Q50 123 72 118" fill="none" stroke="#eed3b1" stroke-width="1" opacity=".3" />
      <path d="M33 148 Q50 154 67 148" fill="none" stroke="#39271b" stroke-width=".8" opacity=".15" />
    </g>

    <g v-else-if="kind === 'vase'">
      <ellipse cx="50" cy="157" rx="31" ry="2" fill="#211910" opacity=".14" />
      <path d="M38 39 C37 55 39 65 33 76 C26 89 16 101 18 123 C20 142 30 155 50 156 C70 155 80 142 82 123 C84 101 74 89 67 76 C61 65 63 55 62 39Z" fill="currentColor" />
      <path d="M38 39 C37 55 39 65 33 76 C26 89 16 101 18 123 C20 142 30 155 50 156 C70 155 80 142 82 123 C84 101 74 89 67 76 C61 65 63 55 62 39Z" :fill="paint('surface')" />
      <ellipse cx="50" cy="39" rx="12" ry="3.5" fill="currentColor" />
      <ellipse cx="50" cy="39" rx="9" ry="2.2" fill="#44382e" opacity=".66" />
      <path d="M38 40 Q50 44 62 40" fill="none" stroke="#f2e3ce" stroke-width="1" opacity=".36" />
      <path d="M29 94 C22 111 23 134 34 144" fill="none" stroke="#fff8eb" stroke-width="2" opacity=".12" />
      <path d="M33 151 Q50 155 67 151" fill="none" stroke="#4b3a2c" stroke-width=".9" opacity=".18" />
      <g fill="none" stroke="#eadcc3" stroke-width=".7" opacity=".12">
        <path d="M23 118 Q50 125 77 118 M24 125 Q50 132 76 125 M27 134 Q50 140 73 134" />
      </g>
    </g>

    <g v-else-if="kind === 'candle'">
      <ellipse cx="50" cy="157" rx="29" ry="2" fill="#211910" opacity=".14" />
      <ellipse cx="50" cy="39" rx="24" ry="28" :fill="paint('glow')" />
      <path d="M50 53 C39 47 43 37 51 24 C50 34 59 39 56 46 C55 50 53 53 50 53Z" :fill="paint('flame')" />
      <path d="M50 51 C46 46 48 41 51 37 C52 42 54 47 50 51Z" fill="#fff0c6" opacity=".86" />
      <path d="M50 56 L50 49" stroke="#483629" stroke-width="1.6" stroke-linecap="round" />
      <path d="M35 59 L35 123 Q50 131 65 123 L65 59Z" fill="currentColor" />
      <path d="M35 59 L35 123 Q50 131 65 123 L65 59Z" :fill="paint('surface')" />
      <ellipse cx="50" cy="59" rx="15" ry="4.5" fill="currentColor" />
      <ellipse cx="50" cy="59" rx="11" ry="2.7" fill="#fff4da" opacity=".36" />
      <path d="M37 60 C40 63 40 71 42 72 C45 72 42 63 47 62" fill="none" stroke="#fff4da" stroke-width="2" opacity=".28" />
      <path d="M31 123 Q50 131 69 123 L67 131 Q50 137 33 131Z" :fill="paint('brass')" />
      <ellipse cx="50" cy="123" rx="19" ry="4.4" :fill="paint('brass')" />
      <path d="M34 122 Q50 127 66 122" fill="none" stroke="#edce93" stroke-width=".9" opacity=".54" />
      <path d="M45 132 L45 145 Q41 149 32 151 L28 154 Q50 159 72 154 L68 151 Q59 149 55 145 L55 132Z" :fill="paint('brass')" />
      <path d="M45 145 Q50 147 55 145 M32 152 Q50 157 68 152" fill="none" stroke="#efdab0" stroke-width=".8" opacity=".28" />
    </g>

    <g v-else>
      <ellipse cx="50" cy="157" rx="39" ry="2" fill="#211910" opacity=".14" />
      <path d="M76 106 L86 155 L74 155 L67 119Z" fill="#5d493a" />
      <path d="M12 54 L85 54 L85 156 L12 156Z" fill="currentColor" />
      <path d="M12 54 L85 54 L85 156 L12 156Z" :fill="paint('surface')" />
      <path d="M16 58 L81 58 L81 152 L16 152Z" fill="#5b4635" opacity=".45" />
      <rect x="19" y="61" width="59" height="87" fill="#eee4d1" />
      <g :clip-path="paint('picture')">
        <rect x="23" y="66" width="55" height="77" :fill="paint('paper')" />
        <circle cx="65" cy="86" r="13" fill="#bf855c" opacity=".75" />
        <path d="M20 130 C32 111 40 114 51 124 C61 134 70 114 82 114 L83 146 L20 146Z" fill="#8c9a7c" />
        <path d="M20 137 C36 127 41 131 49 137 C60 144 68 132 82 126 L83 146 L20 146Z" fill="#647760" />
        <g fill="none" stroke="#445d49" stroke-linecap="round">
          <path d="M43 141 C43 119 41 97 36 78 M43 120 L57 107 M42 109 L31 96 M40 97 L48 85" stroke-width="1.2" />
        </g>
        <g fill="#4f6a51">
          <path d="M36 84 C28 83 28 74 30 70 C36 72 39 77 36 84Z" />
          <path d="M40 101 C36 90 42 83 51 81 C51 90 47 99 40 101Z" />
          <path d="M36 105 C25 106 21 99 23 92 C31 93 37 98 36 105Z" />
          <path d="M45 123 C45 112 51 105 61 105 C60 116 53 123 45 123Z" />
        </g>
        <path d="M23 67 L36 67 L61 143 L48 143Z" fill="#fffdf4" opacity=".1" />
      </g>
      <path d="M13 55 L84 55 M13 55 L13 155" fill="none" stroke="#f4dfbd" stroke-width="1" opacity=".3" />
      <path d="M17 60 L80 60 M17 60 L17 151" fill="none" stroke="#352b21" stroke-width=".8" opacity=".27" />
      <path d="M14 154 L83 154 L83 57" fill="none" stroke="#392d23" stroke-width="1" opacity=".28" />
    </g>
  </svg>
</template>

<style scoped>
.shelf-decoration-art { display: block; width: 100%; height: 100%; max-height: 160px; overflow: visible; }
</style>
