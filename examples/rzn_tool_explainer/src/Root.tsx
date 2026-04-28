import "./index.css";
import { Composition, Folder } from "remotion";
import { RznBrowserExplainer } from "./Composition";

export const RemotionRoot: React.FC = () => {
  return (
    <Folder name="RZN-Marketing">
      <Composition
        id="RznBrowserExplainer"
        component={RznBrowserExplainer}
        durationInFrames={720}
        fps={30}
        width={1920}
        height={1080}
      />
    </Folder>
  );
};
