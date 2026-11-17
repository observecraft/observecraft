import { useEffect, useRef, useState, type InputHTMLAttributes, type Ref } from 'react';

/**
 * A search box whose text lives in the URL (`?q=`). The input keeps its own text: bound straight to the
 * URL, fast typing lost characters, because React Router's `setSearchParams` doesn't queue updates (each
 * keystroke started from the URL before the previous one). The URL is followed again only when it changes
 * from elsewhere (a link, back and forward).
 */
export function UrlSearchInput({
  value,
  onChange,
  placeholder,
  ...rest
}: { value: string; onChange: (text: string) => void; placeholder: string; ref?: Ref<HTMLInputElement> } & Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'>) {
  const [text, setText] = useState(value);
  const written = useRef(value);
  useEffect(() => {
    if (value !== written.current) {
      written.current = value;
      setText(value);
    }
  }, [value]);
  return (
    <input
      {...rest}
      placeholder={placeholder}
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        written.current = e.target.value;
        onChange(e.target.value);
      }}
    />
  );
}
