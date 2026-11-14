// Цветовые схемы из макета: светлая и тёмная для каждой.
export type Tokens = Record<string, string>;
export type Scheme = { name: string; light: Tokens; dark: Tokens };

export const SCHEMES: Record<string, Scheme> = {
  shield: { name:'Север',
    light:{bg:'#EEF2F3',surf:'#F9FBFB',inset:'#E3E9EB',ink:'#15232D',ink2:'#3F4F5A',ink3:'#566671',line:'#CDD6DA',work:'#A64A08',workBg:'#F7E6D8',onWork:'#FFFFFF',own:'#0E5A8A',ownBg:'#DCEAF3',onOwn:'#FFFFFF',pos:'#0B6B57',warn:'#A4262C',warnBg:'#F8E3E1',serOwn:'#0E5A8A',serWork:'#A64A08'},
    dark:{bg:'#141E26',surf:'#1C2933',inset:'#25343F',ink:'#E7EEF1',ink2:'#B4C2CA',ink3:'#93A3AD',line:'#32434F',work:'#F2A25C',workBg:'#3A2A1C',onWork:'#1E1206',own:'#7DBDEB',ownBg:'#1D3445',onOwn:'#0A1C2A',pos:'#5FCFB0',warn:'#F4968E',warnBg:'#43241F',serOwn:'#4C9BD8',serWork:'#CE7A2B'} },
  taiga: { name:'Тайга',
    light:{bg:'#EEF1EA',surf:'#FAFBF6',inset:'#E2E7DB',ink:'#1B2418',ink2:'#434F3E',ink3:'#5B6756',line:'#D0D7C8',work:'#945700',workBg:'#F3E7CE',onWork:'#FFFFFF',own:'#1F6B4F',ownBg:'#D8EBE1',onOwn:'#FFFFFF',pos:'#2C6E2F',warn:'#A32A2A',warnBg:'#F6E1DD',serOwn:'#007F5F',serWork:'#B8601A'},
    dark:{bg:'#151C14',surf:'#1E271C',inset:'#283326',ink:'#E9EFE4',ink2:'#BAC6B3',ink3:'#98A590',line:'#344031',work:'#E2AE55',workBg:'#382C15',onWork:'#1C1405',own:'#78CFA6',ownBg:'#1C3529',onOwn:'#08201A',pos:'#8FD38F',warn:'#F1968F',warnBg:'#40231F',serOwn:'#37A578',serWork:'#BD8620'} },
  baikal: { name:'Байкал',
    light:{bg:'#EAF2F5',surf:'#F8FCFD',inset:'#DCE8ED',ink:'#0F2630',ink2:'#38505C',ink3:'#4F6975',line:'#C5D6DD',work:'#B53B25',workBg:'#F8E2DC',onWork:'#FFFFFF',own:'#00677B',ownBg:'#D2EAF0',onOwn:'#FFFFFF',pos:'#00695C',warn:'#9B1C4A',warnBg:'#F7E0E8',serOwn:'#0A7FA6',serWork:'#B53B25'},
    dark:{bg:'#0F1E26',surf:'#172A34',inset:'#203642',ink:'#E4F0F4',ink2:'#AFC5CE',ink3:'#8CA5B0',line:'#2B4350',work:'#FF9E85',workBg:'#40241C',onWork:'#2A0F08',own:'#5CCBE0',ownBg:'#123643',onOwn:'#04222B',pos:'#5FD3BC',warn:'#F78FB3',warnBg:'#42202E',serOwn:'#1FA5BD',serWork:'#E06A50'} },
  plum: { name:'Слива',
    light:{bg:'#F2EFF4',surf:'#FCFAFD',inset:'#E7E2EC',ink:'#241B2E',ink2:'#4D4358',ink3:'#665B72',line:'#D8D1DF',work:'#A32C6C',workBg:'#F5DFEA',onWork:'#FFFFFF',own:'#4A4AA6',ownBg:'#E2E2F5',onOwn:'#FFFFFF',pos:'#12705F',warn:'#A8321E',warnBg:'#F7E2DC',serOwn:'#4A4AA6',serWork:'#A32C6C'},
    dark:{bg:'#1B1522',surf:'#251D2E',inset:'#30273B',ink:'#EFEAF4',ink2:'#C3B9CF',ink3:'#A296AF',line:'#3C3248',work:'#F28BBF',workBg:'#43213A',onWork:'#2B0A1E',own:'#A3A6F5',ownBg:'#2B2A50',onOwn:'#14133A',pos:'#66D3BD',warn:'#F59C8A',warnBg:'#45261F',serOwn:'#7F83E8',serWork:'#E55C9F'} },
  graphite: { name:'Графит',
    light:{bg:'#F0F0EF',surf:'#FBFBFA',inset:'#E5E5E3',ink:'#1C1C1B',ink2:'#474745',ink3:'#5F5F5C',line:'#D5D5D2',work:'#8A5B00',workBg:'#F1E6CF',onWork:'#FFFFFF',own:'#33404A',ownBg:'#DFE4E8',onOwn:'#FFFFFF',pos:'#256B50',warn:'#A32A2A',warnBg:'#F6E1DD',serOwn:'#2F72A8',serWork:'#966400'},
    dark:{bg:'#191918',surf:'#232322',inset:'#2D2D2B',ink:'#EDEDEB',ink2:'#BFBFBB',ink3:'#9E9E9A',line:'#3A3A38',work:'#E6B450',workBg:'#3A2F17',onWork:'#1C1503',own:'#B4C3CF',ownBg:'#2B3238',onOwn:'#12181D',pos:'#78CFA6',warn:'#F1968F',warnBg:'#40231F',serOwn:'#4F98D4',serWork:'#BB8A24'} }
};

export type ThemeMode = 'light' | 'dark' | 'system';

export function applyTheme(schemeId: string, mode: ThemeMode): 'light' | 'dark' {
  let resolved: 'light' | 'dark' = mode === 'dark' ? 'dark' : 'light';
  if (mode === 'system') {
    try {
      resolved = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    } catch {
      resolved = 'light';
    }
  }
  const t = (SCHEMES[schemeId] || SCHEMES.shield)[resolved];
  const root = document.documentElement;
  for (const k of Object.keys(t)) root.style.setProperty('--' + k, t[k]);
  root.style.setProperty('--scrim', resolved === 'dark' ? 'rgba(0,0,0,.6)' : 'rgba(15,25,32,.45)');
  root.style.colorScheme = resolved;
  return resolved;
}
